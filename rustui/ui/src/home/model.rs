//! Home's model (`LumaHome.qml`'s logic, pure): the merged theme and layout, the feeds the cards read, the dock, the focus (the kit's
//! [`Focus`] registry over the status bar, the cards and the dock, with the layers that open over Home as modal scopes), the keys (OK on
//! release, a hold for a tile's options or the layout editor, Back one step at a time, a digit opens a dock tile), the pointer (hover
//! is a cue, a click selects), and what Home asks the host for: luna calls (the launch points, launches), the files (`layout.json`,
//! `theme.json`, `tint.json` read every two seconds and written by the editor), the helper (a profile switch, a card's action).

use super::apps::Apps;
use super::dock::{APPS_TILE, Dock};
use super::editor::{Editor, EditorOut};
use super::grid::{self, Entry, Rect};
use super::picker::{Picker, PickerOut};
use super::radar_zoom::{KEY_STEP, WHEEL_STEP};
use super::popover::{self, Done, Popover};
use super::search::{Search, SearchOut};
use super::onhome::layout::Metrics;
use super::onhome::smart::{Out as SmartOut, Smart};
use super::wallvideo::{Ctx, WallVideo};
use crate::kit::focus::{Dir, Focus, Id, Moved, Region, Scope};
use crate::kit::screen::{Effect, Input};
use crate::panels::hub;
use crate::system::extras::{Intent, Note, Page, Pick, Tile};
use serde_json::{Value, json};

pub const STATUS: Region = Region(1);
pub const WIDGETS: Region = Region(2);
pub const DOCK: Region = Region(3);
pub const STATUS_ID: u32 = 100;
/// How far the status bar's buttons are from the pill's ends and edges: half of what the pill is taller than a button, so that every button is concentric with the pill's cap.
pub const STATUS_INSET: f32 = 6.0;

/// What a button of the status bar is for. The bar reads left to right in this order, and a change of it is a wider gap: the actions the owner
/// takes, then who is watching. Read-only indicators would come before both; the bar has none (the clock is a card, the network and the receiver
/// are Quick Settings', the volume and the mute are the overlays').
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Action,
    Identity,
}

impl Purpose {
    /// The purpose of the button of layout type `kind`.
    pub fn of(kind: &str) -> Self {
        if kind == "profile" { Purpose::Identity } else { Purpose::Action }
    }
}

/// The type of a status bar item of the layout (`profile`, `notifications`, `settings`, `search`, `cameras`).
fn kind_of(item: &Value) -> &str {
    item.get("type").and_then(Value::as_str).unwrap_or("")
}

/// The question over Home: the two answers' focus ids and region.
pub const CONFIRM_ID: u32 = 960;
pub const CONFIRM_REGION: Region = Region(31);
/// The question's answers, the safe one first, and whether the answer is drawn as danger.
pub const CONFIRM_ANSWERS: [(&str, bool); 2] = [("Keep it", false), ("Uninstall", true)];
pub const CONFIRM_BODY: &str = "The app and its data on this TV are removed. It can be installed again from where it came from.";
pub const WIDGET_ID: u32 = 200;
pub const DOCK_ID: u32 = 300;
pub const DOCK_MAX: u32 = 60;

pub const SCOPE_APPS: Scope = Scope(1);
pub const SCOPE_SEARCH: Scope = Scope(2);
pub const SCOPE_EDIT: Scope = Scope(3);
pub const SCOPE_PICKER: Scope = Scope(4);
pub const SCOPE_MENU: Scope = Scope(6);
pub const SCOPE_CONFIRM: Scope = Scope(7);
pub const SCOPE_WEATHER: Scope = Scope(8);
pub const SCOPE_ONHOME: Scope = Scope(9);

/// The luna tags Home calls with.
pub const LP_TAG: u64 = 300;
pub const LP_FALLBACK_TAG: u64 = 301;
pub const LAUNCH_TAG: u64 = 302;
/// The file reads.
pub const THEME_TAG: u64 = 310;
/// The write of what the status bar's settings popover changed (`wallpaperMotion`).
pub const POPOVER_TAG: u64 = 314;
pub const LAYOUT_TAG: u64 = 311;
/// What the shell answers a poll of the theme or the layout with when the file is as it was last read (the HTTP status's meaning).
pub const NOT_MODIFIED: u16 = 304;
pub const TINT_TAG: u64 = 312;
pub const SWITCH_TAG: u64 = 320;
/// The answer of a control of a smart-home tile (`POST /v1/onhome/act`).
pub const ONHOME_TAG: u64 = 330;
/// The read of the token the helper keeps for the shell to unlock with (`onhome/shell.json`), asked only after the person pressed Unlock.
pub const UNLOCK_TAG: u64 = 331;
/// The answer of a change of the owner's list (`POST /v1/onhome/picks`: a size, a place, hide).
pub const PICKS_TAG: u64 = 332;
/// The read of a sensor's history (`GET /v1/onhome/history`).
pub const HISTORY_TAG: u64 = 333;
/// The answers of the controls an item sends: the number says which, so that a refusal takes back that item's state and no other's.
pub const ACT_TAG: u64 = 3_300_000;
/// The read of the profile kept from the last time the helper answered (`last-profile.json`).
pub const KEPT_PROFILE_TAG: u64 = 322;
/// The write of Quick Access's choice (`remote.quickAccess`).
pub const ACCESS_TAG: u64 = 313;

const POLL_MS: u64 = 2000;
const APP_MGR: &str = "luna://com.webos.applicationManager";
const HLP: &str = "luna://com.webos.service.homelaunchpoints";

/// What is open over Home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    None,
    Apps,
    Search,
    Edit,
    Picker,
    /// The options of a tile (`docs/mocks/16-context-menu`): the rows of [`Home::menu_rows`].
    Menu,
    /// The question that follows Uninstall (`panels/Confirm.qml`): Keep it, the safe answer and the one Back gives, or Uninstall.
    Confirm,
    /// The status bar's settings popover: the wallpaper's motion in live rows ([`super::popover`]).
    Popover,
    /// The full weather view ([`super::weather_view`]): OK on the weather card.
    Weather,
    /// One smart-home tile's controls ([`super::onhome::Sheet`]): a held OK on the tile.
    OnHome,
}

/// What the pointer holds on a lamp's colour sheet while its button is down.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Held {
    /// The ring of hues.
    Ring,
    /// A track.
    Track(super::onhome::colour::Row),
}

/// A row of the tile menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub id: &'static str,
    pub label: String,
    pub danger: bool,
}

/// The question over Home that follows Uninstall.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirm {
    pub app_id: String,
    pub title: String,
    pub icon: String,
    /// 0 Keep it, 1 Uninstall.
    pub index: usize,
}

/// What Home asks another kind of the shell for (the status bar's gear and bell, the Menu key's quick panel): `shell.rs` routes it to
/// the kind that owns the surface when that kind is in the list, and logs it otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Quick,
    /// The app switcher: Back from the top of Home (when the owner wants it) and a press of Home with nothing open on Home.
    Switcher,
    Settings,
    /// Settings at one of its pages (Quick Access's More, a Search answer that is a page).
    SettingsPage(String),
    Notifications,
    Cameras,
    /// The camera viewer at full size, for the camera of the cameras topic with this id (OK on a camera item of the smart-home card), opened
    /// on the picture the card shows of it.
    CameraFull(String),
    /// The camera list that grows out of the status bar's camera button (OK on it): every camera with a small live picture, one of them to open at full size.
    CameraList,
}

/// The one thing a view needs that is not read from the model: where the focus is now and whether it has rested.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusView {
    pub id: Option<Id>,
    pub rested: bool,
    pub hover: Option<Id>,
}

/// The profile chip on the status bar, for a button height `h`: a circular avatar set in the pill's left cap with an even `inset` all
/// round (the avatar and the cap share a centre), then the name, then the cap's end. The measures follow the height, so they grow with
/// the text size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chip {
    pub inset: f32,
    /// The avatar's diameter.
    pub avatar: f32,
    /// Where the name's box begins, from the chip's left edge.
    pub name_x: f32,
    pub width: f32,
}

impl Chip {
    /// With the name's measured width, or none (the avatar alone, a round button).
    pub fn new(h: f32, name_width: Option<f32>) -> Self {
        let inset = (h / 12.0).round();
        let avatar = h - 2.0 * inset;
        let name_x = inset + avatar + (h * 0.2).round();
        match name_width {
            Some(w) => Self { inset, avatar, name_x, width: name_x + w + (h * 0.35).round() },
            None => Self { inset, avatar, name_x, width: h },
        }
    }
}

/// The drawn layout of a layout revision and a text scale (its bits).
type DrawnLayout = ((u64, u32), std::rc::Rc<Value>);

pub struct Home {
    pub theme: Value,
    pub layout: Value,
    /// Counts the layouts that differ from the one before and the themes whose `clock` or `tiles` do: what the surface's chrome is made from.
    layout_rev: u64,
    theme_chrome_rev: u64,
    /// The layout as it is drawn ([`Home::drawn_layout`]) for the layout revision and text scale it was made at.
    drawn: std::cell::RefCell<Option<DrawnLayout>>,
    pub shipped_layout: Value,
    pub layout_text: String,
    pub theme_text: String,
    theme_preview: Option<Value>,
    pub tint: Value,
    pub weather: Value,
    /// The helper's `weather.more`: the hours, the details of now and the alerts of the full weather view.
    pub weather_more: Value,
    pub wview: super::weather_view::WeatherView,
    /// The weather view's radar page: the helper's frames, the loop, the scrub.
    pub radar: super::radar_view::Radar,
    /// Whether motion is reduced (the surface says): the radar loop then does not play by itself.
    pub motion_reduced: bool,
    /// What a layer's close asked of the helper, sent with the next update.
    deferred: Vec<Effect>,
    pub news: Value,
    /// The helper's `plugins` roster, its `ha.link` and the plugins' `plugin.<id>.cards`: what the At-home card is drawn from.
    pub plugins: hub::Topics,
    pub cameras: Value,
    /// The helper's `onhome` topic: what Home Assistant shares with the TV, as tiles in words.
    pub onhome: Value,
    /// What the last refused control said, and until when the card shows it.
    onhome_said: Option<(String, u64)>,
    /// The controls of the tile that is open, while its sheet is.
    pub sheet: Option<super::onhome::Sheet>,
    /// A sensor's history, while its sheet is open.
    pub history: Option<super::onhome::sensor::SensorSheet>,
    /// A lamp's colours, while its sheet is open.
    pub colour: Option<super::onhome::colour::ColourSheet>,
    /// What the pointer holds on the colour sheet while its button is down: the ring or a track, which follows it wherever it goes.
    colour_held: Option<Held>,
    /// A thermostat's set point and modes, while its sheet is open.
    pub thermostat: Option<super::onhome::thermostat::ThermostatSheet>,
    /// Where the pointer was last seen, for a turn of the wheel to find what it is over.
    pointer_at: (f32, f32),
    /// One state of the smart-home card for each card of the layout that is one: the items, which is in front, what the person asked for.
    pub smarts: Vec<(String, Smart)>,
    /// The measures of the card's type, which the surface takes from the kit (until it has, the shipped scale's).
    pub onhome_metrics: Metrics,
    /// The controls on their way: the tag, the card and the control's number.
    acts: Vec<(u64, String, u64)>,
    act_tag: u64,
    /// OK went down with the pointer on an item: the card, the item, when, and whether the hold has already done what it does.
    item_press: Option<(String, usize, u64, bool)>,
    /// The lock the person said to unlock, while the token to do it with is being read.
    unlock_of: Option<String>,
    pub stills: super::stills::Stills,
    /// What the bell's badge counts (the panels' log and the plugins' notes since the drawer was last open).
    pub unread: usize,
    pub profiles: Value,
    /// The active profile as the helper last reported it, kept in `last-profile.json`: the chip keeps the person's letter, colour and
    /// name while the helper is stopped or starting again.
    kept_profile: Option<Value>,
    pub voice_ready: bool,
    pub helper_up: bool,
    pub dock: Dock,
    pub focus: Focus,
    pub apps: Apps,
    pub search: Search,
    pub editor: Option<Editor>,
    pub picker: Picker,
    pub layer: Layer,
    /// A layer under the open one (search over the library).
    stack: Vec<Layer>,
    pub menu: Vec<MenuRow>,
    pub menu_index: usize,
    pub menu_app: Option<Value>,
    /// The question over Home while it is up: the app, and which answer has the focus (0 Keep it, 1 Uninstall).
    pub confirm: Option<Confirm>,
    /// The status bar's settings popover while it is up, and what it changed that is to be written to the theme at the next update.
    pub popover: Option<Popover>,
    wallpaper_return_settings: bool,
    /// Where the popover and its rows are (made when it opens and when its focus moves; the surface draws this).
    pub popover_lay: Option<popover::Layout>,
    persist: Option<Value>,
    pub inner: Option<String>,
    pub inner_index: Option<usize>,
    pointer_down: bool,
    /// A visible sample with the button up has been seen since the pointer last showed: only then is a button down a press seen (a button already down
    /// as the pointer appears, or a release with no press, is nothing).
    pointer_armed: bool,
    /// What the press of a click landed on: the click is made when the same button is released over it.
    click_press: Option<(Id, u64)>,
    pub active: bool,
    /// The clip of the moving wallpaper is on the plane under Home and the hole is open: Home's own picture and scrim are away ([`WallVideo::up`]).
    pub wall_hole: bool,
    /// What the compositor's `wall` topic says: the hole of the wallpaper's window is under Home.
    wall_topic: bool,
    /// The moving wallpaper's controller: asks the player for the clip while Home wants it.
    pub wall: WallVideo,
    light: bool,
    saver_up: bool,
    /// Home is to be calm (`Shell::after`): something is over it, or a key was pressed a moment ago. What moves on its own is not looked at then, and
    /// costs: the GPU wallpaper steps ten times a second and each step owes the whole screen to every page of the ring (round 22's ring at 120 Hz:
    /// three frames of 157 layers, 13-16 ms of GPU each, under Settings), and the weather's sprites make a frame every refresh, which keeps the
    /// compositor drawing in step with the panel for the cheapest frame it has seen (round 22, 60 Hz: the heavy frame a key asks for then misses its
    /// refresh, and the screen is never quiet for the gate to see a burst end). A camera's still is one of them: every second the card is drawn again in
    /// a frame of its own, 24 layers of the floor and 2-3 ms of the shell's thread on the host with two cameras, in the middle of a panel's slide or a
    /// page being made.
    calm: bool,
    /// Settings, the display lab or the guide covers the screen (`Shell::after`): Home is seen only dimmed, through the glass of its sheets. Its
    /// cards' contents are brought up to date once a second then (`HomeSurface::contents_due`), not at each change of the sheet: a page being made
    /// asked for them again at each of the helper's answers, 0.7-1.3 ms of the shell's thread apiece on the host.
    covered: bool,
    pub screen_on: bool,
    pub cursor_visible: bool,
    pub launching: Option<String>,
    ok_down_at: Option<u64>,
    ok_held: bool,
    moved_at: u64,
    /// The redraw that shows what waits for the focus to rest (the dock tile's name) has been asked for since the last move.
    rest_drawn: bool,
    rest_ms: u64,
    hold_ms: u64,
    next_poll: u64,
    poll_outstanding: u8,
    started: bool,
    pub now: u64,
    /// The widget ids in the order their focus ids were given, so an id is stable while the layout changes.
    widget_ids: Vec<String>,
    switched_at: Option<u64>,
    dirty: bool,
    text_scale: f32,
    /// The width the chip's name takes in its type, measured by the surface (the model has no fonts): the name and text size it was
    /// measured for, and the width. Until then the chip is laid out on an estimate.
    name_width: Option<(String, f32, f32)>,
    /// How far apart the status bar's buttons are within a purpose and between two: the theme's `space(2)` and `space(5)`.
    status_gaps: (f32, f32),
    requests: Vec<Request>,
    /// What the extras Home draws around itself are told (an app opens, a digit did something, the remote key, a story opens).
    notes: Vec<Note>,
    /// The Settings pages Search may answer with (given by the shell, which has Settings), and whether Search was opened and wants them again.
    search_pages: Vec<Value>,
    pages_wanted: bool,
}

fn merge_over(base: &Value, over: &Value) -> Value {
    let mut out = base.clone();
    lumatv_tokens::merge(&mut out, over);
    out
}

/// How far apart the status bar's buttons are: within a purpose `space(2)`, between two purposes `space(5)` (4, 8, 12, 16, 20, 24, 32, 40 in the shipped
/// theme), so the rhythm follows the theme's spacing scale.
fn status_gaps(theme: &Value) -> (f32, f32) {
    let tokens = lumatv_tokens::Tokens::from_json(theme);
    (tokens.space(2), tokens.space(5))
}

impl Home {
    /// `theme` is the merged theme (the shipped under the owner's), `shipped_layout` the shipped layout and `user_layout` the owner's file.
    pub fn new(theme: Value, shipped_layout: Value, user_layout: Value) -> Self {
        let motion = theme.get("motion").cloned().unwrap_or(Value::Null);
        let rest_ms = motion.get("restMs").and_then(Value::as_u64).unwrap_or(150);
        let hold_ms = motion.get("holdMs").and_then(Value::as_u64).unwrap_or(700);
        let mut h = Self {
            layout: Value::Null,
            layout_rev: 0,
            theme_chrome_rev: 0,
            drawn: std::cell::RefCell::new(None),
            layout_text: serde_json::to_string(&user_layout).unwrap_or_default(),
            theme_text: String::new(),
            theme_preview: None,
            tint: Value::Null,
            weather: Value::Null,
            weather_more: Value::Null,
            wview: super::weather_view::WeatherView::default(),
            radar: super::radar_view::Radar::default(),
            motion_reduced: false,
            deferred: Vec::new(),
            news: Value::Null,
            plugins: Default::default(),
            voice_ready: false,
            cameras: Value::Null,
            onhome: Value::Null,
            onhome_said: None,
            sheet: None,
            history: None,
            colour: None,
            colour_held: None,
            thermostat: None,
            pointer_at: (0.0, 0.0),
            smarts: Vec::new(),
            onhome_metrics: Metrics::nominal(1.0),
            acts: Vec::new(),
            act_tag: 0,
            item_press: None,
            unlock_of: None,
            stills: super::stills::Stills::default(),
            unread: 0,
            profiles: Value::Null,
            kept_profile: None,
            inner: None,
            inner_index: None,
            pointer_down: false,
            pointer_armed: false,
            click_press: None,
            helper_up: false,
            dock: Dock::default(),
            focus: Focus::new(),
            apps: Apps::default(),
            search: Search::default(),
            editor: None,
            picker: Picker::default(),
            layer: Layer::None,
            stack: Vec::new(),
            menu: Vec::new(),
            menu_index: 0,
            menu_app: None,
            confirm: None,
            popover: None,
            wallpaper_return_settings: false,
            popover_lay: None,
            persist: None,
            active: false,
            wall_hole: false,
            wall_topic: false,
            wall: WallVideo::default(),
            light: false,
            saver_up: false,
            calm: false,
            covered: false,
            screen_on: true,
            cursor_visible: false,
            launching: None,
            ok_down_at: None,
            ok_held: false,
            moved_at: 0,
            rest_drawn: true,
            rest_ms,
            hold_ms,
            next_poll: 0,
            poll_outstanding: 0,
            started: false,
            now: 0,
            widget_ids: Vec::new(),
            switched_at: None,
            dirty: true,
            text_scale: 1.0,
            name_width: None,
            status_gaps: status_gaps(&theme),
            requests: Vec::new(),
            notes: Vec::new(),
            search_pages: Vec::new(),
            pages_wanted: false,
            theme,
            shipped_layout,
        };
        h.apply_layout(user_layout);
        h
    }

    // ---- the layout and the theme
    /// The merged layout from the owner's file, tidied; the cards and the dock placed.
    pub fn apply_layout(&mut self, user: Value) {
        let merged = merge_over(&self.shipped_layout, &user);
        self.set_layout(grid::tidy_layout(&merged));
        for e in self.laid_out() {
            if !self.widget_ids.contains(&e.id) {
                self.widget_ids.push(e.id.clone());
            }
        }
        if let Some(d) = self.dock_entry() {
            let r = grid::rect_of(&self.layout, &d);
            let entry = grid::widgets(&self.layout).iter().find(|w| w.get("id").and_then(Value::as_str) == Some(&d.id)).cloned().unwrap_or(Value::Null);
            self.dock.configure(&entry, r.w, r.h);
        }
        self.register();
        self.dirty = true;
    }

    /// Settings changed the layout (a card shown, sized or moved, or put back to the shipped one): written as the owner's file and drawn at once.
    pub fn set_layout_from_settings(&mut self, layout: &Value, out: &mut Vec<Effect>) {
        let file = grid::for_file(layout, &self.shipped_layout);
        let text = serde_json::to_string_pretty(&file).unwrap_or_default() + "\n";
        self.layout_text = file.to_string();
        out.push(Effect::Write { path: "theme/layout.json".into(), text });
        self.apply_layout(file);
        self.redraw(out);
    }

    /// Home draws `layout` now (the editor's preview); nothing is written.
    pub fn preview_layout(&mut self, layout: Value) {
        self.set_layout(layout);
        self.register();
        self.dirty = true;
    }

    fn set_layout(&mut self, layout: Value) {
        if layout != self.layout {
            self.layout_rev += 1;
        }
        self.layout = layout;
    }

    /// What the surface's chrome (the cards, the clock, the tiles' look) is made from: moves when the layout, or the theme's `clock` or `tiles`, changes.
    pub fn chrome_rev(&self) -> (u64, u64) {
        (self.layout_rev, self.theme_chrome_rev)
    }

    pub fn set_theme(&mut self, theme: Value) {
        if theme.get("clock") != self.theme.get("clock") || theme.get("tiles") != self.theme.get("tiles") {
            self.theme_chrome_rev += 1;
        }
        self.status_gaps = status_gaps(&theme);
        self.theme = theme;
        self.dock.refresh(&self.theme);
        self.rest_ms = self.theme.pointer("/motion/restMs").and_then(Value::as_u64).unwrap_or(150);
        self.hold_ms = self.theme.pointer("/motion/holdMs").and_then(Value::as_u64).unwrap_or(700);
        self.search.configure(&self.theme, self.text_scale);
        self.register();
        if self.layer == Layer::Search { self.search.register(&mut self.focus); }
        self.dirty = true;
    }

    /// Keep disk polling from replacing a temporary Settings preview.
    pub fn set_theme_preview(&mut self, preview: Option<Value>) {
        self.theme_preview = preview;
    }

    /// The cards on Home (not the dock, not hidden), in the layout's order; the cameras' card only while the Cameras plugin is on.
    pub fn cards(&self) -> Vec<Entry> {
        let mut cards = self.laid_out();
        if self.cameras_off() {
            cards.retain(|e| e.kind != "cameras");
        }
        cards
    }

    /// Every card the layout places (not the dock, not hidden), drawn now or not.
    fn laid_out(&self) -> Vec<Entry> {
        grid::entries(&self.layout).into_iter().filter(|e| !e.is_dock() && !e.hidden).collect()
    }

    /// The helper says the Cameras plugin is switched off; no word from it is not off.
    fn cameras_off(&self) -> bool {
        self.cameras.get("state").and_then(Value::as_str) == Some("off")
    }

    pub fn dock_entry(&self) -> Option<Entry> {
        grid::entries(&self.layout).into_iter().find(Entry::is_dock)
    }

    /// The layout as it is drawn: the status bar at the text size's height. Every card's rectangle is read from it, dozens of times an apply and
    /// again for each card of the report, and each was a copy of the whole layout (the report took 0.19 ms of the host's thread of which 0.14 ms
    /// were these); it is made again when the layout, which only `set_layout` changes, or the text size does.
    fn drawn_layout(&self) -> std::rc::Rc<Value> {
        let key = (self.layout_rev, self.text_scale.to_bits());
        if let Some((made, drawn)) = &*self.drawn.borrow()
            && *made == key
        {
            return drawn.clone();
        }
        let drawn = std::rc::Rc::new(grid::with_status_height(&self.layout, self.status_height()));
        *self.drawn.borrow_mut() = Some((key, drawn.clone()));
        drawn
    }

    pub fn rect(&self, e: &Entry) -> Rect {
        grid::rect_of(&self.drawn_layout(), e)
    }

    pub fn dock_rect(&self) -> Option<Rect> {
        self.dock_entry().map(|d| self.rect(&d))
    }

    /// The status bar's items in the order they are drawn, left to right, as `{type, icon}` objects: the actions, then who is watching.
    pub fn status_items(&self) -> Vec<Value> {
        self.status_item_list().cloned().collect()
    }

    /// [`Self::status_items`] without copying them; the cameras' button only while the Cameras plugin is on.
    fn status_item_list(&self) -> impl Iterator<Item = &Value> {
        let cameras_off = self.cameras_off();
        self.layout
            .pointer("/statusBar/items")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .filter(move |it| !(cameras_off && kind_of(it) == "cameras"))
    }

    /// The place of the button of type `kind` among the status bar's items (its focus id is `STATUS_ID` plus it).
    pub fn status_index(&self, kind: &str) -> Option<usize> {
        self.status_item_list().position(|it| kind_of(it) == kind)
    }

    pub fn safe(&self) -> (f32, f32) {
        (self.layout.pointer("/safe/x").and_then(Value::as_f64).unwrap_or(64.0) as f32, self.layout.pointer("/safe/y").and_then(Value::as_f64).unwrap_or(54.0) as f32)
    }

    /// The bar's height grows with the text size (the layout's 60 at 100 %), as the chip's name does. It is an even number of pixels, so that the buttons
    /// (the pill's height less `STATUS_INSET` either side) are even too, and a glyph or a lift centred on a button has its edges on whole pixels.
    pub fn status_height(&self) -> f32 {
        let h = self.layout.pointer("/statusBar/height").and_then(Value::as_f64).unwrap_or(60.0) as f32 * self.text_scale.max(1.0);
        (h / 2.0).round() * 2.0
    }

    /// The bell's count, from the panels' notification log; true when the badge has to be drawn again.
    pub fn set_unread(&mut self, n: usize) -> bool {
        if n == self.unread {
            return false;
        }
        self.unread = n;
        self.dirty = true;
        true
    }

    /// The person's text size (the kit's), which the chrome's own sizes follow.
    pub fn set_text_scale(&mut self, scale: f32) {
        if (scale - self.text_scale).abs() > 0.001 {
            self.text_scale = scale;
            self.onhome_metrics = Metrics::nominal(scale);
            self.apps.scale = scale;
            self.search.configure(&self.theme, scale);
            self.register();
            if self.layer == Layer::Search { self.search.register(&mut self.focus); }
            self.dirty = true;
        }
    }

    /// The focus id of a card.
    pub fn widget_id(&self, id: &str) -> Option<Id> {
        self.widget_ids.iter().position(|w| w == id).map(|i| Id(WIDGET_ID + i as u32))
    }

    pub fn widget_of(&self, id: Id) -> Option<&str> {
        let i = id.0.checked_sub(WIDGET_ID)? as usize;
        self.widget_ids.get(i).map(String::as_str)
    }

    /// Where each button is along the bar, from the first one's left edge: its offset and its width. Buttons of one purpose are `status_gaps.0`
    /// apart, a change of purpose `status_gaps.1`.
    fn status_run(&self) -> Vec<(f32, f32)> {
        let mut out = Vec::new();
        let mut x = 0.0;
        let mut before: Option<Purpose> = None;
        for it in self.status_item_list() {
            let purpose = Purpose::of(kind_of(it));
            if let Some(b) = before {
                x += if b == purpose { self.status_gaps.0 } else { self.status_gaps.1 };
            }
            let w = self.status_item_width(it);
            out.push((x, w));
            x += w;
            before = Some(purpose);
        }
        out
    }

    /// The status bar's two boxes: the one its buttons sit in (as wide as they are, `STATUS_INSET` round them) and the glass behind them. Both are
    /// placed along the bar's row by the registry's `statusbar` item (`layout.overlays.statusbar` of the theme, read each time: the middle of the
    /// screen until the owner moves it). With `full` there the glass spreads over the whole row, from the cards' grid's left edge to its right,
    /// and the buttons stay where the placement puts them inside it.
    fn status_boxes(&self) -> (ui::geometry::Frame, ui::geometry::Frame) {
        const ITEM: &str = "statusbar";
        let span = self.status_run().last().map_or(0.0, |(x, w)| x + w) + 2.0 * STATUS_INSET;
        let h = self.status_height();
        let frame = lumatv_placement::Frame::from_json(&self.layout, self.theme.pointer("/overlays/limit"));
        let registry = lumatv_placement::Registry::shipped();
        let user = registry.user_from_json(self.theme.pointer("/layout/overlays").unwrap_or(&Value::Null));
        let size = lumatv_placement::Size::new(span.round() as i32, h.round() as i32);
        let group = registry.rect_of(ITEM, &user, &frame, size).expect("the status bar is an item of the registry");
        let glass = registry.glass_of(ITEM, &user, &frame, size).expect("the status bar is an item of the registry");
        let spread = registry.placement_of(ITEM, &user).is_some_and(|p| p.placement.full);
        (ui::geometry::Frame::new(group.x as f32, group.y as f32, span, h), ui::geometry::Frame::new(glass.x as f32, glass.y as f32, if spread { glass.w as f32 } else { span }, h))
    }

    /// The glass the status bar's buttons sit on: as wide as they are, or the whole row of the cards' grid with Full width on.
    pub fn status_pill(&self) -> ui::geometry::Frame {
        self.status_boxes().1
    }

    /// The box the status bar's buttons are in, `STATUS_INSET` round them: where the placement puts them, inside the glass.
    pub fn status_group(&self) -> ui::geometry::Frame {
        self.status_boxes().0
    }

    /// The row the status bar is in, edge to edge of the safe area: what a popover under a button keeps inside.
    pub fn status_row(&self) -> ui::geometry::Frame {
        let (sx, sy) = self.safe();
        ui::geometry::Frame::new(sx, sy, 1920.0 - 2.0 * sx, self.status_height())
    }

    /// The status bar's buttons in their box, one square of the box's inner height each (the profile chip adds its name): `(x, y, w, h)` in the
    /// order of [`Self::status_items`], which is the order the D-pad walks them in.
    pub fn status_frames(&self) -> Vec<(f32, f32, f32, f32)> {
        let group = self.status_group();
        let item_h = self.status_height() - 2.0 * STATUS_INSET;
        self.status_run().into_iter().map(|(x, w)| (group.x + STATUS_INSET + x, group.y + STATUS_INSET, w, item_h)).collect()
    }

    /// A button is a square of the pill's inner height; the profile chip adds its name.
    pub fn status_item_width(&self, item: &Value) -> f32 {
        let h = self.status_height() - 2.0 * STATUS_INSET;
        if kind_of(item) == "profile" { self.chip().width } else { h }
    }

    /// The profile chip's measures at the status bar's button height.
    pub fn chip(&self) -> Chip {
        let name = self.profile_name();
        let h = self.status_height() - 2.0 * STATUS_INSET;
        if name.is_empty() {
            return Chip::new(h, None);
        }
        let scale = self.text_scale.max(1.0);
        let w = match &self.name_width {
            Some((n, s, w)) if *n == name && (*s - scale).abs() < 0.001 => *w,
            _ => 9.0 * scale * name.chars().count() as f32,
        };
        // the measured width is a fraction of a pixel: the chip is as wide as the whole pixels the name needs, so that the pill and every button beside it stand on whole pixels
        Chip::new(h, Some(w.ceil()))
    }

    /// The name the surface has to measure (it has the fonts): the chip's, when no width for it at this text size is held yet.
    pub fn name_to_measure(&self) -> Option<String> {
        let name = self.profile_name();
        let scale = self.text_scale.max(1.0);
        let held = matches!(&self.name_width, Some((n, s, _)) if *n == name && (*s - scale).abs() < 0.001);
        (!name.is_empty() && !held).then_some(name)
    }

    /// The measured width of `name` at this text size: the chip, and the buttons after it, move to it.
    pub fn set_name_width(&mut self, name: String, width: f32) {
        self.name_width = Some((name, self.text_scale.max(1.0), width));
        self.register();
        self.dirty = true;
    }

    /// Where the button of type `kind` is on the status bar (`[x, y, w, h]`), as the bar lays it out now: the place a panel grows out of and goes
    /// back into. With no such button in the layout, the middle of the pill.
    fn status_frame_of(&self, kind: &str) -> [f32; 4] {
        let at = self.status_index(kind);
        if let Some(&(x, y, w, h)) = at.and_then(|i| self.status_frames().get(i).copied()).as_ref() {
            return [x, y, w, h];
        }
        let group = self.status_group();
        let side = self.status_height() - 2.0 * STATUS_INSET;
        [(group.x + (group.w - side) / 2.0).round(), group.y + STATUS_INSET, side, side]
    }

    /// The bell: the notification drawer's way in and out.
    pub fn bell_frame(&self) -> [f32; 4] {
        self.status_frame_of("notifications")
    }

    /// The camera button: the camera list's way in and out.
    pub fn camera_frame(&self) -> [f32; 4] {
        self.status_frame_of("cameras")
    }

    /// The gear: Quick Settings' way in and out.
    pub fn gear_frame(&self) -> [f32; 4] {
        self.status_frame_of("settings")
    }

    /// The profile chip: the way in and out of Who's watching.
    pub fn chip_frame(&self) -> [f32; 4] {
        self.status_frame_of("profile")
    }

    /// Whether the focus is on the camera button: the camera list asks for its first pictures once the focus has rested there.
    pub fn camera_focused(&self) -> bool {
        self.active && self.layer == Layer::None && self.status_index("cameras").is_some_and(|i| self.focus.current() == Some(Id(STATUS_ID + i as u32)))
    }

    // ---- profiles
    pub fn profile(&self) -> Option<&Value> {
        self.live_profile().or(self.kept_profile.as_ref())
    }

    fn live_profile(&self) -> Option<&Value> {
        if self.profiles.get("ready").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        let active = self.profiles.get("active").and_then(Value::as_str)?;
        self.profiles.get("profiles").and_then(Value::as_array)?.iter().find(|p| p.get("id").and_then(Value::as_str) == Some(active))
    }

    pub fn profile_name(&self) -> String {
        self.profile().and_then(|p| p.get("name")).and_then(Value::as_str).unwrap_or("").to_owned()
    }

    /// The focusables of this layout, registered with the focus engine (an id is kept across layouts; a card that went is gone).
    fn register(&mut self) {
        let frames = self.status_frames();
        let items = self.status_items();
        for i in 0..32u32 {
            let id = Id(STATUS_ID + i);
            match frames.get(i as usize) {
                Some(&(x, y, w, h)) if i < items.len() as u32 => self.focus.register(id, ui::geometry::Frame::new(x, y, w, h), STATUS, Scope::BASE),
                _ => self.focus.unregister(id),
            }
        }
        let cards = self.cards();
        for (i, wid) in self.widget_ids.clone().iter().enumerate() {
            let id = Id(WIDGET_ID + i as u32);
            match cards.iter().find(|c| &c.id == wid) {
                Some(e) => {
                    let r = self.rect(e);
                    self.focus.register(id, ui::geometry::Frame::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32), WIDGETS, Scope::BASE);
                }
                None => self.focus.unregister(id),
            }
        }
        self.register_dock();
        self.sync_smarts();
    }

    /// The dock's tiles as focusables where they are drawn now (the grid scrolled to its row).
    fn register_dock(&mut self) {
        let Some(r) = self.dock_rect() else {
            for i in 0..DOCK_MAX {
                self.focus.unregister(Id(DOCK_ID + i));
            }
            return;
        };
        let scroll = self.dock.scroll_px();
        for i in 0..DOCK_MAX {
            let id = Id(DOCK_ID + i);
            if (i as usize) < self.dock.points.len() {
                let (sx, sy) = self.dock.slot(i as usize);
                let frame = ui::geometry::Frame::new(r.x as f32 + sx as f32, r.y as f32 + sy as f32 - scroll, self.dock.tile as f32, self.dock.tile as f32);
                self.focus.register(id, frame, DOCK, Scope::BASE);
                // a tile that is scrolled out of the card is reached by the arrows, not the pointer: its frame is where it will be
                self.focus.set_enabled(id, true);
            } else {
                self.focus.unregister(id);
            }
        }
    }

    fn dock_index_of(id: Id) -> Option<usize> {
        id.0.checked_sub(DOCK_ID).filter(|i| *i < DOCK_MAX).map(|i| i as usize)
    }

    // ---- what goes out
    pub fn start(&mut self, now: u64) -> Vec<Effect> {
        self.started = true;
        self.now = now;
        self.next_poll = now + POLL_MS;
        let mut out = vec![
            Effect::Luna { tag: LP_TAG, uri: format!("{HLP}/listLaunchPoints"), payload: json!({"subscribe": true}), subscribe: true },
            Effect::Luna { tag: LP_FALLBACK_TAG, uri: format!("{APP_MGR}/listLaunchPoints"), payload: json!({}), subscribe: false },
            Effect::Read { tag: TINT_TAG, path: "theme/tint.json".into() },
            Effect::Read { tag: KEPT_PROFILE_TAG, path: "last-profile.json".into() },
        ];
        self.focus.ensure();
        out.push(Effect::Redraw);
        out
    }

    fn redraw(&mut self, out: &mut Vec<Effect>) {
        if !out.contains(&Effect::Redraw) {
            out.push(Effect::Redraw);
        }
        self.dirty = true;
    }

    pub fn update(&mut self, input: &Input, now: u64) -> Vec<Effect> {
        self.now = now;
        let mut out = Vec::new();
        match input {
            Input::Topic { topic, data } => self.topic(topic, data, &mut out),
            Input::Luna { tag, payload } => self.luna(*tag, payload, &mut out),
            Input::Helper { topic, data } => self.helper(topic, data, &mut out),
            Input::Fetched { tag, status, data } => self.fetched(*tag, *status, data, &mut out),
            Input::Key { name, down, repeat } => self.key(name, *down, *repeat, &mut out),
            Input::Pointer { x, y, buttons, visible } => self.pointer(*x, *y, *buttons, *visible, &mut out),
            Input::Tick | Input::Bytes { .. } => {}
        }
        // the radar page's answers (the state, the frames, the map) and its loop
        if self.radar.open {
            let loops = self.radar_loops();
            let (fx, changed) = match input {
                Input::Fetched { .. } | Input::Bytes { .. } => self.radar.input(input, now),
                _ => self.radar.tick(now, loops),
            };
            out.extend(fx);
            if changed {
                self.redraw(&mut out);
            }
        }
        out.append(&mut self.deferred);
        self.tick(&mut out);
        let visible = self.camera_stills_visible();
        out.extend(self.stills.update(input, visible, now));
        out.extend(self.wall.update(input, now));
        self.sync_wall(&mut out);
        // what the popover changed is written when it has closed
        if let Some(patch) = self.persist.take() {
            out.push(Effect::Theme { tag: POPOVER_TAG, patch: json!({"wallpaperMotion": patch}) });
        }
        out
    }

    /// Fits the moving wallpaper's controller to what Home is now (the theme, the material, whether it is the front, the saver, the hole under it); the
    /// pictures Home draws itself go while the clip is on the plane.
    fn sync_wall(&mut self, out: &mut Vec<Effect>) {
        let ctx = Ctx { dark: !self.light, active: self.active && self.screen_on, saver: self.saver_up };
        out.extend(self.wall.sync(&self.theme, ctx, self.wall_topic, self.now));
        let up = self.wall.up();
        if up != self.wall_hole {
            self.wall_hole = up;
            self.redraw(out);
        }
    }

    /// The material in use and whether the screensaver is over Home: what the clip's contract reads besides the theme.
    pub fn set_context(&mut self, light: bool, saver_up: bool, now: u64) -> Vec<Effect> {
        let mut out = Vec::new();
        if light != self.light || saver_up != self.saver_up {
            self.light = light;
            self.saver_up = saver_up;
            self.now = now;
            self.sync_wall(&mut out);
        }
        out
    }

    /// Whether Home is to be calm ([`Home`]'s `calm`). Returns whether it changed, so that Home is drawn again.
    pub fn set_calm(&mut self, calm: bool) -> bool {
        let changed = calm != self.calm;
        self.calm = calm;
        changed
    }

    /// Whether a full-screen sheet is over Home ([`Home`]'s `covered`).
    pub fn set_covered(&mut self, covered: bool) {
        self.covered = covered;
    }

    pub fn covered(&self) -> bool {
        self.covered
    }

    /// Whether the GPU wallpaper stands still.
    pub fn wallpaper_held(&self) -> bool {
        self.calm
    }

    fn camera_stills_visible(&self) -> bool {
        self.active && self.screen_on && self.layer == Layer::None && !self.calm && self.cameras.get("state").and_then(Value::as_str) == Some("ready")
    }

    fn tick(&mut self, out: &mut Vec<Effect>) {
        let now = self.now;
        if let Some(at) = self.ok_down_at
            && !self.ok_held
            && now >= at + self.hold_ms
            && self.hold_applies()
        {
            self.ok_held = true;
            self.hold(out);
        }
        // the pointer held on an item as long as OK is held: what a held OK does, and its release is no click
        if let Some((wid, _, at, false)) = self.item_press.clone()
            && now >= at + self.hold_ms
            && self.smart_of(&wid).is_some_and(Smart::can_hold)
        {
            if let Some(p) = self.item_press.as_mut() {
                p.3 = true;
            }
            self.smart_hold_of(&wid, out);
            self.redraw(out);
        }
        // a button held as long as OK is held (`LuPointer`'s press and hold): the tile's options or the editor, and its release is no click
        if let Some((_, at)) = self.click_press
            && now >= at + self.hold_ms
        {
            self.click_press = None;
            self.hold(out);
            self.redraw(out);
        }
        // the focus has rested: what waited for it (the dock tile's name) is drawn now
        if !self.rest_drawn && now >= self.moved_at + self.rest_ms {
            self.rest_drawn = true;
            self.redraw(out);
        }
        // an answer that never came (the host restarted) does not stop the next look
        if self.poll_outstanding > 0 && now >= self.next_poll + 2 * POLL_MS {
            self.poll_outstanding = 0;
        }
        if self.started && self.active && now >= self.next_poll && self.poll_outstanding == 0 {
            self.next_poll = now + POLL_MS;
            self.poll_outstanding = 2;
            out.push(Effect::Read { tag: THEME_TAG, path: "theme/theme.json".into() });
            out.push(Effect::Read { tag: LAYOUT_TAG, path: "theme/layout.json".into() });
        }
        if let Some(ed) = &mut self.editor {
            if ed.tick(now) {
                self.redraw(out);
            }
        }
        if self.picker.tick(now) {
            self.redraw(out);
        }
        if self.layer == Layer::Search && self.search.tick(now) {
            self.redraw(out);
        }
        if self.onhome_said.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.onhome_said = None;
            self.redraw(out);
        }
        // what a card shows in place of what the topic says (a state asked for, a size tried) goes when its time is up
        let mut changed = false;
        for (_, s) in &mut self.smarts {
            changed |= s.tick(now);
        }
        if changed {
            self.redraw(out);
        }
        if let Some(sheet) = self.sheet.as_mut() {
            let due = sheet.due(now);
            if sheet.tick(now) || due.is_some() {
                self.redraw(out);
            }
            if let Some(act) = due {
                self.send_act(&act, out);
            }
        }
        self.send_colour(out);
        self.send_thermostat(out);
    }

    pub fn due_at(&self) -> Option<u64> {
        let mut due: Option<u64> = None;
        let mut take = |t: Option<u64>| {
            if let Some(t) = t {
                due = Some(due.map_or(t, |d| d.min(t)));
            }
        };
        if let Some((_, at)) = self.click_press {
            take(Some(at + self.hold_ms));
        }
        if let Some(at) = self.ok_down_at.filter(|_| !self.ok_held && self.hold_applies()) {
            take(Some(at + self.hold_ms));
        }
        if let Some((wid, _, at, false)) = &self.item_press
            && self.smart_of(wid).is_some_and(Smart::can_hold)
        {
            take(Some(at + self.hold_ms));
        }
        for (_, s) in &self.smarts {
            take(s.due_at());
        }
        if self.moved_at > 0 && self.now < self.moved_at + self.rest_ms {
            take(Some(self.moved_at + self.rest_ms));
        }
        if self.started && self.active {
            take(Some(self.next_poll));
        }
        take(self.editor.as_ref().and_then(Editor::due_at));
        take(self.picker.due_at());
        if self.layer == Layer::Search { take(self.search.due_at()); }
        take(self.stills.due_at(self.camera_stills_visible()));
        take(self.wall.due_at());
        take(self.radar.due_at(self.radar_loops()));
        take(self.onhome_said.as_ref().map(|(_, until)| *until));
        if let Some(sheet) = &self.sheet {
            take(sheet.waiting());
            take(sheet.notice.as_ref().map(|(_, until)| *until));
        }
        take(self.colour.as_ref().and_then(|c| c.due_at()));
        take(self.thermostat.as_ref().and_then(|t| t.due_at()));
        // the clock's minute, and its second hand when the analogue face shows one: only while Home is in front (behind an app nothing shows it)
        if self.active {
            let step = if self.theme.pointer("/clock/style").and_then(Value::as_str) == Some("analog") { 1000 } else { 60_000 };
            take(Some((self.now / step + 1) * step));
        }
        due
    }

    fn topic(&mut self, topic: &str, data: &Value, out: &mut Vec<Effect>) {
        match topic {
            "foreground" => {
                let home = data.get("home").and_then(Value::as_bool).unwrap_or(false);
                let covered = data.get("covered").and_then(Value::as_bool).unwrap_or(!home);
                let active = home && !covered;
                if active != self.active {
                    self.active = active;
                    if active {
                        self.launching = None;
                        self.focus.ensure();
                        self.next_poll = self.now;
                        self.wall.read_manifest(self.now);
                    } else {
                        self.reset_ok();
                        self.close_all();
                    }
                    self.redraw(out);
                }
            }
            "wall" => {
                self.wall_topic = data.get("hole").and_then(Value::as_bool).unwrap_or(false);
            }
            "screen" => {
                let on = data.get("on").and_then(Value::as_bool).unwrap_or(true);
                // "Who's watching?" on wake, when the owner asked for it and Home is in front: the picker is resident, so it shows
                // on the next frame
                if on && !self.screen_on && self.picker.picker_on_wake && self.active && self.layer == Layer::None {
                    self.open_picker(out);
                }
                self.screen_on = on;
            }
            // the Home key while Home is in front: with nothing open on Home and the switcher on offer (the compositor says so), the app switcher
            "homekey" => {
                if data.get("switcher").and_then(Value::as_bool) == Some(true) && self.at_rest() {
                    self.requests.push(Request::Switcher);
                }
            }
            // the keyboard hook: text typed by another keyboard (the Panels' own, a phone) lands in the search field while it is up;
            // a trailing newline is Done
            "keyboard.text" => {
                if self.layer == Layer::Search
                    && let Some(text) = data.get("text").and_then(Value::as_str)
                {
                    let text = text.to_owned();
                    let o = self.search.hook_text(&text);
                    self.search_out(o, out);
                    // the answers the text has made are keys' stops now
                    if self.layer == Layer::Search {
                        self.search.register(&mut self.focus);
                    }
                    self.redraw(out);
                }
            }
            "cursor" => {
                let v = data.get("visible").and_then(Value::as_bool).unwrap_or(false);
                if v != self.cursor_visible {
                    self.cursor_visible = v;
                    if !v {
                        self.focus.pointer_gone();
                        self.redraw(out);
                    }
                }
            }
            _ => {}
        }
    }

    fn luna(&mut self, tag: u64, payload: &Value, out: &mut Vec<Effect>) {
        match tag {
            LP_TAG | LP_FALLBACK_TAG => {
                if let Some(list) = payload.get("launchPoints").and_then(Value::as_array) {
                    if tag == LP_FALLBACK_TAG && !self.dock.points.is_empty() {
                        return;
                    }
                    if self.dock.moving {
                        return;
                    }
                    self.dock.set_points(list, &self.theme);
                    self.apps.set_points(&self.dock, &self.theme);
                    self.register_dock();
                    self.redraw(out);
                }
            }
            _ => {}
        }
    }

    fn helper(&mut self, topic: &str, data: &Value, out: &mut Vec<Effect>) {
        match topic {
            "up" => self.helper_up = data.as_bool().unwrap_or(false),
            "weather" => self.weather = data.clone(),
            "weather.more" => {
                self.weather_more = data.clone();
                if self.layer == Layer::Weather {
                    self.redraw(out);
                }
            }
            "news" => self.news = data.clone(),
            "plugins" | "ha.link" => {
                self.plugins.insert(topic.to_owned(), data.clone());
            }
            t if t.starts_with("plugin.") && t.ends_with(".cards") => {
                self.plugins.insert(topic.to_owned(), data.clone());
            }
            "cameras" => {
                let was_off = self.cameras_off();
                self.cameras = data.clone();
                self.feed_smarts();
                // the Cameras plugin was switched: its status button and its card come or go
                if self.cameras_off() != was_off {
                    self.layout_rev += 1;
                    self.register();
                    self.redraw(out);
                }
            }
            "onhome" => {
                self.onhome = data.clone();
                self.feed_smarts();
                // the thing whose sheet is open is gone from Home: the sheet goes with it; a lamp's colours are the sheet's own again once the
                // person lets go of them
                if self.layer == Layer::OnHome {
                    if self.onhome_sheet_gone() {
                        self.close_layer();
                    } else if let Some(tile) = self.sheet_tile() {
                        let now = self.now;
                        if let Some(c) = self.colour.as_mut() {
                            c.sync(&tile, now);
                        }
                        if let Some(t) = self.thermostat.as_mut() {
                            t.sync(&tile, now);
                        }
                    }
                }
                self.redraw(out);
            }
            "voice" => {
                self.voice_ready = data.get("ready").and_then(Value::as_bool) == Some(true);
                // words heard open the search with them (`voiceFill`), or fill the sheet that is up
                if data.get("state").and_then(Value::as_str) == Some("heard")
                    && let Some(text) = data.get("text").and_then(Value::as_str).filter(|t| !t.is_empty())
                {
                    let text = text.to_owned();
                    if self.layer != Layer::Search && self.active {
                        self.open_search(out);
                    }
                    if self.layer == Layer::Search {
                        let o = self.search.hook_text(&text);
                        self.search_out(o, out);
                        self.search.register(&mut self.focus);
                    }
                }
            }
            "profiles" => {
                let before = self.switched_at;
                let was = self.profiles.get("active").cloned();
                self.profiles = data.clone();
                if let Some(live) = self.live_profile().cloned()
                    && self.kept_profile.as_ref() != Some(&live)
                {
                    out.push(Effect::Write { path: "last-profile.json".into(), text: live.to_string() });
                    self.kept_profile = Some(live);
                }
                self.switched_at = data.get("switchedAt").and_then(Value::as_u64);
                self.picker.set_profiles(data);
                // a change of person: the working copy is the new person's, read at once; what was open closes
                if before.is_some() && self.switched_at != before && was != data.get("active").cloned() {
                    self.close_all();
                    self.next_poll = self.now;
                    self.poll_outstanding = 0;
                    self.picker.switched(self.now);
                }
                self.register();
            }
            _ => return,
        }
        self.redraw(out);
    }

    fn fetched(&mut self, tag: u64, status: u16, data: &Value, out: &mut Vec<Effect>) {
        match tag {
            THEME_TAG => {
                self.poll_outstanding = self.poll_outstanding.saturating_sub(1);
                if status == 200 {
                    let text = data.to_string();
                    if text != self.theme_text {
                        self.theme_text = text;
                        let mut file = data.clone();
                        if let Some(preview) = &self.theme_preview {
                            crate::host::theme::merge_one_level(&mut file, preview);
                        }
                        let merged = lumatv_tokens::Tokens::from_json(&file).json().clone();
                        self.set_theme(merged);
                        self.redraw(out);
                    }
                }
            }
            LAYOUT_TAG => {
                self.poll_outstanding = self.poll_outstanding.saturating_sub(1);
                if status == NOT_MODIFIED {
                    return;
                }
                let text = if status == 200 { data.to_string() } else { String::new() };
                if self.editor.is_none() && text != self.layout_text {
                    self.layout_text = text;
                    self.apply_layout(if status == 200 { data.clone() } else { json!({}) });
                    self.redraw(out);
                }
            }
            KEPT_PROFILE_TAG if status == 200 => {
                // only what the helper has not said yet: a live profile is newer than the file
                if self.kept_profile.is_none() && data.get("id").is_some() {
                    self.kept_profile = Some(data.clone());
                    self.register();
                    self.redraw(out);
                }
            }
            TINT_TAG if status == 200 => {
                self.tint = data.clone();
                self.redraw(out);
            }
            SWITCH_TAG if self.picker.answered(status, data, self.now) => self.redraw(out),
            UNLOCK_TAG => {
                // the helper's token for the shell, read now that the person said Unlock: the one request that unlocks
                let id = self.unlock_of.take();
                match (id, data.get("token").and_then(Value::as_str).filter(|_| status == 200)) {
                    (Some(id), Some(token)) => out.push(Effect::Request { tag: ONHOME_TAG, method: "POST".into(), path: "/v1/onhome/unlock".into(), body: Some(json!({"id": id, "token": token})) }),
                    (Some(_), None) => {
                        self.onhome_said = Some(("The TV cannot unlock from here".into(), self.now + 4000));
                        self.redraw(out);
                    }
                    _ => {}
                }
            }
            HISTORY_TAG => {
                if let Some(h) = self.history.as_mut()
                    && h.arrived(status, data)
                {
                    self.redraw(out);
                }
            }
            ONHOME_TAG if status != 200 => {
                let words = data.pointer("/error/message").and_then(Value::as_str).filter(|w| !w.is_empty()).unwrap_or("Home Assistant did not do that").to_owned();
                self.onhome_said = Some((words.clone(), self.now + 4000));
                if let Some(sheet) = self.sheet.as_mut() {
                    sheet.say(&words, self.now);
                }
                self.redraw(out);
            }
            t if (ACT_TAG..ACT_TAG + 0x1_0000).contains(&t) => self.act_answered(t, status, data, out),
            PICKS_TAG if status != 200 => {
                // the arrangement that was shown at once is not the owner's: the list comes back as the helper has it
                for (_, s) in &mut self.smarts {
                    s.forget_arrangement();
                }
                let words = data.pointer("/error/message").and_then(Value::as_str).filter(|w| !w.is_empty()).unwrap_or("The list was not changed").to_owned();
                self.onhome_said = Some((words, self.now + 4000));
                self.redraw(out);
            }
            _ => {}
        }
    }

    // ---- keys
    fn dir_of(name: &str) -> Option<Dir> {
        Some(match name {
            "up" => Dir::Up,
            "down" => Dir::Down,
            "left" => Dir::Left,
            "right" => Dir::Right,
            _ => return None,
        })
    }

    /// A kind above Home owned this key or pointer sample: what Home held of it is let go, so a press it saw before the owner opened is not
    /// a hold or a click that comes after the owner has closed.
    pub fn input_taken(&mut self, input: &Input) {
        match input {
            Input::Key { name, .. } if name == "ok" => self.reset_ok(),
            Input::Pointer { .. } => {
                self.pointer_down = false;
                self.click_press = None;
            }
            _ => {}
        }
    }

    fn reset_ok(&mut self) {
        self.ok_down_at = None;
        self.ok_held = false;
        if self.dock.moving {
            self.dock.cancel_move();
            self.register_dock();
        }
    }

    pub fn key(&mut self, name: &str, down: bool, repeat: bool, out: &mut Vec<Effect>) {
        if !self.active {
            if name == "ok" && !down {
                self.reset_ok();
            }
            return;
        }
        // a notch of the Magic Remote's wheel, with its direction: the smart-home card under the pointer scrolls
        if name == "wheelup" || name == "wheeldown" {
            if down && !repeat {
                self.wheel(name == "wheeldown", out);
            }
            return;
        }
        if self.layer == Layer::Search {
            if down { self.focus.key(); }
            let before = self.search.visual_state();
            let result = self.search.input(name, down, repeat, &mut self.focus, self.now);
            self.search_out(result, out);
            if self.layer != Layer::Search || self.search.visual_state() != before { self.redraw(out); }
            return;
        }
        if let Some(d) = name.chars().next().filter(|c| c.is_ascii_digit() && name.len() == 1) {
            if down && !repeat {
                if self.layer == Layer::Picker {
                    self.picker_key(name, None, out);
                    self.redraw(out);
                } else if self.layer != Layer::Weather {
                    self.digit(d, out);
                }
            }
            return;
        }
        // the on-screen remote's key: it opens over Home with nothing else open (`LumaHome.qml`'s `remoteKeyPressed`)
        if name == "screenremote" {
            if down && !repeat && self.layer == Layer::None && self.editor.is_none() && self.inner.is_none() && !self.dock.moving {
                self.notes.push(Note::Remote);
                self.redraw(out);
            }
            return;
        }
        if name == "ok" {
            if down {
                if repeat || self.ok_down_at.is_some() {
                    return;
                }
                self.ok_down_at = Some(self.now);
                self.ok_held = false;
                // the item in front is pressed before its press lands: the card shows it in this frame (a sheet over the card has the keys, and
                // an OK in it presses nothing under it: the press would outlive the sheet and land on the item it was left on, not the one in front then)
                if let Some(wid) = self.onhome_inner().filter(|_| self.layer == Layer::None) {
                    let now = self.now;
                    if let Some(s) = self.smart_mut(&wid) {
                        s.press_down(now);
                    }
                    self.redraw(out);
                }
                return;
            }
            // a release with no press of Home's before it is the end of a press that another kind took (it closed as the press came): no OK
            let Some(at) = self.ok_down_at else {
                return;
            };
            // a release that comes after the hold's time without a tick in between (a slow frame) is the hold's release
            if !self.ok_held && self.now >= at + self.hold_ms && self.hold_applies() {
                self.ok_held = true;
                self.hold(out);
            }
            let held = self.ok_held;
            self.ok_down_at = None;
            self.ok_held = false;
            if held {
                self.redraw(out);
                return;
            }
            self.focus.key();
            self.action("ok", out);
            return;
        }
        if !down {
            return;
        }
        if repeat && name == "back" {
            return;
        }
        if name == "back" || name == "menu" || name == "search" {
            self.focus.key();
        }
        self.action(name, out);
    }

    /// A press of OK held `hold_ms`: a tile's options on the dock and in the library, the layout editor on a card.
    fn hold(&mut self, out: &mut Vec<Effect>) {
        match self.layer {
            Layer::Apps => {
                if let Some(app) = self.apps.focused_app() {
                    self.open_menu(app, "library", out);
                }
            }
            Layer::None if self.onhome_inner().is_some() => self.smart_hold(out),
            Layer::None => match self.focus.current() {
                Some(id) if Self::dock_index_of(id).is_some() => {
                    if !self.dock.moving {
                        self.open_tile_menu(out);
                    }
                }
                Some(id) if self.widget_of(id).is_some() => {
                    let wid = self.widget_of(id).unwrap_or("").to_owned();
                    self.edit_home(&wid, out);
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn action(&mut self, name: &str, out: &mut Vec<Effect>) {
        self.moved_at = self.now;
        self.rest_drawn = false;
        let dir = Self::dir_of(name);
        match self.layer {
            Layer::Apps => self.apps_key(name, dir, out),
            Layer::Search => self.search_key(name, dir, out),
            Layer::Edit => self.edit_key(name, dir, out),
            Layer::Picker => self.picker_key(name, dir, out),
            Layer::Menu => self.menu_key(name, dir, out),
            Layer::Confirm => self.confirm_key(name, dir, out),
            Layer::Popover => self.popover_key(name, dir, out),
            Layer::Weather => self.weather_key(name, dir, out),
            Layer::OnHome => self.sheet_key(name, out),
            Layer::None => self.home_key(name, dir, out),
        }
        self.redraw(out);
    }

    fn home_key(&mut self, name: &str, dir: Option<Dir>, out: &mut Vec<Effect>) {
        if name == "menu" {
            if let Some(wid) = self.onhome_inner() {
                // the item in front has its own menu (size, place, hide)
                self.smart_key(&wid, name, out);
            } else if self.focus.current().is_some_and(|id| Self::dock_index_of(id).is_some()) {
                self.open_tile_menu(out);
            } else {
                self.open_apps(out);
            }
            return;
        }
        if name == "search" {
            self.open_search(out);
            return;
        }
        // a card's inner focus: the content has the keys until Back
        if let Some(wid) = self.inner.clone() {
            if self.kind_of(&wid) == "onhome" {
                self.smart_key(&wid, name, out);
                return;
            }
            if name == "back" || !self.inner_key(&wid, name, out) {
                self.inner = None;
                self.inner_index = None;
            }
            return;
        }
        let Some(cur) = self.focus.current().or_else(|| self.focus.ensure()) else { return };
        if let Some(i) = Self::dock_index_of(cur) {
            self.dock.index = i;
            if self.dock.moving {
                match (name, dir) {
                    ("ok", _) => {
                        if let Some(order) = self.dock.end_move() {
                            self.keep_apps(json!({"dockOrder": order}), out);
                        }
                    }
                    ("back", _) => self.dock.cancel_move(),
                    (_, Some(d)) => {
                        let (dx, dy) = match d {
                            Dir::Left => (-1, 0),
                            Dir::Right => (1, 0),
                            Dir::Up => (0, -1),
                            Dir::Down => (0, 1),
                        };
                        self.dock.move_held(dx, dy);
                    }
                    _ => {}
                }
                self.register_dock();
                let id = Id(DOCK_ID + self.dock.index as u32);
                self.focus.focus(id);
                return;
            }
            match (name, dir) {
                ("ok", _) => self.activate_tile(out),
                ("back", _) => {
                    let x = self.dock_rect().map_or(0.0, |r| r.x as f32) + self.dock.focus_centre_x();
                    self.focus_lowest_card_by_x(x);
                }
                (_, Some(d)) => {
                    let roll = match d {
                        Dir::Left => self.dock.rolled(-1),
                        Dir::Right => self.dock.rolled(1),
                        _ => None,
                    };
                    if let Some(i) = roll {
                        // past a row's end the focus goes to the next row's first tile; the dock scrolls to it and the glide eases on the shared flow
                        self.focus.key();
                        self.dock.focus(i);
                        self.register_dock();
                        self.focus.focus(Id(DOCK_ID + i as u32));
                    } else if let Moved::To(_, to) = self.focus.step(d)
                        && let Some(i) = Self::dock_index_of(to)
                    {
                        self.dock.focus(i);
                        self.register_dock();
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some(wid) = self.widget_of(cur).map(str::to_owned) {
            match (name, dir) {
                ("ok", _) => {
                    if self.has_inner(&wid) {
                        if self.kind_of(&wid) == "onhome"
                            && let Some(s) = self.smart_mut(&wid)
                        {
                            s.enter();
                        }
                        self.inner = Some(wid);
                    } else {
                        self.activate_card(&wid, out);
                    }
                }
                ("back", _) => self.requests.push(Request::Switcher),
                (_, Some(d)) => {
                    if let Moved::To(_, to) = self.focus.step(d)
                        && let Some(i) = Self::dock_index_of(to)
                    {
                        self.dock.focus(i);
                        self.register_dock();
                    }
                }
                _ => {}
            }
            return;
        }
        // the status bar
        match (name, dir) {
            ("ok", _) => {
                let i = (cur.0 - STATUS_ID) as usize;
                let kind = self.status_items().get(i).and_then(|it| it.get("type")).and_then(Value::as_str).unwrap_or("").to_owned();
                match kind.as_str() {
                    "profile" => self.open_picker(out),
                    "search" => self.open_search(out),
                    "settings" => self.requests.push(Request::Quick),
                    "notifications" => self.requests.push(Request::Notifications),
                    "cameras" => self.requests.push(Request::CameraList),
                    _ => {}
                }
            }
            ("back", _) => self.requests.push(Request::Switcher),
            (_, Some(d)) => {
                if let Moved::To(_, to) = self.focus.step(d)
                    && let Some(i) = Self::dock_index_of(to)
                {
                    self.dock.focus(i);
                    self.register_dock();
                }
            }
            _ => {}
        }
    }

    /// Entering the cards from the dock: the card whose bottom edge is nearest the dock, by x.
    fn focus_lowest_card_by_x(&mut self, x: f32) {
        let area = grid::area(&self.drawn_layout());
        let mut best: Option<(f32, Id)> = None;
        for e in self.cards() {
            let r = self.rect(&e);
            let score = ((r.x + r.w / 2) as f32 - x).abs() + ((area.y + area.h) - (r.y + r.h)) as f32 * 2.0;
            if let Some(id) = self.widget_id(&e.id)
                && best.is_none_or(|b| score < b.0)
            {
                best = Some((score, id));
            }
        }
        if let Some((_, id)) = best {
            self.focus.focus(id);
        }
    }

    /// Quick Access (`remote.quickAccess.on`): a digit says what it opens in a toast, and one with nothing behind it asks what to give it.
    fn access_on(&self) -> bool {
        self.theme.pointer("/remote/quickAccess/on").and_then(Value::as_bool) == Some(true)
    }

    /// The app the owner gave `digit` (`remote.quickAccess.slots`): it comes before the dock's tile of that number.
    fn access_slot(&self, digit: &str) -> Option<String> {
        self.theme.pointer(&format!("/remote/quickAccess/slots/{digit}")).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned)
    }

    /// The apps no digit opens yet, for the question: the dock's order then the rest by name, hidden ones left out, at most `n`.
    fn access_picks(&self, n: usize) -> Vec<Pick> {
        let mut used: Vec<String> = self.theme.pointer("/remote/quickAccess/slots").and_then(Value::as_object).map(|o| o.values().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
        used.extend(self.dock.ids().into_iter().take(10));
        self.apps.all().iter().filter(|a| !a.hidden && !used.contains(&a.id)).take(n).map(|a| Pick { id: a.id.clone(), title: a.title.clone(), icon: a.icon.clone() }).collect()
    }

    fn digit(&mut self, d: char, out: &mut Vec<Effect>) {
        if self.layer != Layer::None || self.theme.pointer("/remote/numberKeys").and_then(Value::as_bool) == Some(false) || self.dock.moving {
            return;
        }
        let digit = d.to_string();
        let i = if d == '0' { 9 } else { (d as u8 - b'1') as usize };
        if self.access_on() {
            if let Some(app) = self.access_slot(&digit).and_then(|id| self.apps.by_id(&id).cloned()) {
                self.launch(&app.id, &app.params, &app.title, out);
                self.notes.push(Note::Toast { digit, title: app.title, line: "Opening.".into() });
                self.redraw(out);
                return;
            }
            if self.access_slot(&digit).is_none() && i >= self.dock.points.len() {
                self.notes.push(Note::Ask { digit, picks: self.access_picks(4) });
                self.redraw(out);
                return;
            }
        }
        if !self.dock.focus(i) {
            return;
        }
        self.register_dock();
        self.focus.focus(Id(DOCK_ID + i as u32));
        let title = self.dock.focused().map(|p| if p.title.is_empty() { p.id.clone() } else { p.title.clone() }).unwrap_or_default();
        self.activate_tile(out);
        // the Apps tile opens the library, which is not a launch to say anything about
        if self.access_on() && self.layer == Layer::None {
            self.notes.push(Note::Toast { digit, title, line: "Opening.".into() });
        }
        self.redraw(out);
    }

    fn activate_tile(&mut self, out: &mut Vec<Effect>) {
        let Some(p) = self.dock.focused().cloned() else { return };
        if p.id == APPS_TILE {
            self.open_apps(out);
            return;
        }
        self.launch(&p.id, &p.params, &p.title, out);
    }

    /// Opens an app: the launch screen is told what it is and where its tile is, and the layers over Home close.
    fn launch(&mut self, id: &str, params: &Value, title: &str, out: &mut Vec<Effect>) {
        let icon = self.dock.points.iter().find(|p| p.id == id).map(|p| p.icon.clone()).or_else(|| self.apps.by_id(id).map(|a| a.icon.clone())).unwrap_or_default();
        let tile = self.launch_tile(id);
        self.notes.push(Note::Launch { id: id.to_owned(), title: if title.is_empty() { id.to_owned() } else { title.to_owned() }, icon, tile });
        self.start_app(id, params, title, out);
    }

    /// Opens an app with no launch screen (the web search and the browser: nothing of LumaTV's own to show while it opens).
    fn start_app(&mut self, id: &str, params: &Value, title: &str, out: &mut Vec<Effect>) {
        self.launching = Some(title.to_owned());
        self.close_all();
        out.push(Effect::Luna { tag: LAUNCH_TAG, uri: format!("{APP_MGR}/launch"), payload: json!({"id": id, "params": params}), subscribe: false });
        out.push(Effect::Log(format!("launch {id}")));
    }

    /// Where the dock's tile of `id` is on the screen when the card of its app is to grow out of it: the tile that has the focus is lifted by
    /// `focus.tileScale` about its centre, and the card grows out of what is drawn, not out of its place. None for an app that is not on the dock
    /// or whose row is scrolled out of the card.
    fn launch_tile(&self, id: &str) -> Option<Tile> {
        let i = self.dock.points.iter().position(|p| p.id == id)?;
        let row = self.dock.row_of(i);
        if row < self.dock.scroll_row || row >= self.dock.scroll_row + self.dock.visible_rows() {
            return None;
        }
        let card = self.dock_rect()?;
        let tokens = lumatv_tokens::Tokens::from_json(&self.theme);
        let lifted = self.layer == Layer::None && self.focus.current() == Some(Id(DOCK_ID + i as u32));
        let k = if lifted { tokens.focus.tile_scale } else { 1.0 };
        let (sx, sy) = self.dock.slot(i);
        let side = self.dock.tile as f32;
        let (cx, cy) = (card.x as f32 + sx as f32 + side / 2.0, card.y as f32 + sy as f32 - self.dock.scroll_px() + side / 2.0);
        Some(Tile { x: cx - side * k / 2.0, y: cy - side * k / 2.0, w: side * k, h: side * k, radius: tokens.radius(lumatv_tokens::Radius::Tile) * k })
    }

    /// What the extras asked of Home: a pick of the question is kept as the digit's app and opens, More is Settings at its remote page.
    pub fn extras_intent(&mut self, intent: Intent, out: &mut Vec<Effect>) {
        match intent {
            Intent::Assign { digit, app } => {
                let mut slots = self.theme.pointer("/remote/quickAccess/slots").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
                slots[&digit] = json!(app);
                let cfg = json!({"on": self.access_on(), "slots": slots});
                if !self.theme.get("remote").is_some_and(Value::is_object) {
                    self.theme["remote"] = json!({});
                }
                self.theme["remote"]["quickAccess"] = cfg.clone();
                out.push(Effect::Theme { tag: ACCESS_TAG, patch: json!({"remote": {"quickAccess": cfg}}) });
                if let Some(a) = self.apps.by_id(&app).cloned() {
                    self.launch(&a.id, &a.params, &a.title, out);
                    self.notes.push(Note::Toast { digit, title: a.title, line: "Assigned. Opening.".into() });
                }
            }
            Intent::More(_) => self.requests.push(Request::SettingsPage("remote".into())),
        }
        self.redraw(out);
    }

    /// Whether Search was just opened and wants the Settings pages (the shell answers with [`Home::set_search_pages`]).
    pub fn take_pages_wanted(&mut self) -> bool {
        std::mem::take(&mut self.pages_wanted)
    }

    /// The Settings pages Search may answer with: kept, and given to the sheet when it is up.
    pub fn set_search_pages(&mut self, pages: Vec<Value>) {
        self.search.set_pages(pages.clone());
        self.search_pages = pages;
        if self.layer == Layer::Search { self.search.register(&mut self.focus); }
        self.dirty = true;
    }

    /// What the extras are to be told since the last take.
    pub fn take_notes(&mut self) -> Vec<Note> {
        std::mem::take(&mut self.notes)
    }

    fn keep_apps(&mut self, part: Value, out: &mut Vec<Effect>) {
        let mut apps = self.theme.get("apps").cloned().unwrap_or_else(|| json!({}));
        lumatv_tokens::merge(&mut apps, &part);
        self.theme["apps"] = apps.clone();
        self.dock.refresh(&self.theme);
        self.apps.set_points(&self.dock, &self.theme);
        self.register_dock();
        // the owner's file plus the patch, one level deep
        let mut file: Value = serde_json::from_str(&self.theme_text).unwrap_or_else(|_| json!({}));
        if !file.is_object() {
            file = json!({});
        }
        let mut fa = file.get("apps").cloned().unwrap_or_else(|| json!({}));
        lumatv_tokens::merge(&mut fa, &part);
        file["apps"] = fa;
        let text = serde_json::to_string_pretty(&file).unwrap_or_default() + "\n";
        self.theme_text = file.to_string();
        out.push(Effect::Write { path: "theme/theme.json".into(), text });
    }

    // ---- cards
    fn has_inner(&self, wid: &str) -> bool {
        match self.cards().iter().find(|e| e.id == wid).map(|e| e.kind.as_str()) {
            // the plugins' chips take the keys only while there are some; with none the card opens Home Assistant's page
            Some("homeassistant") => !self.ha_items().is_empty(),
            Some("news") => true,
            Some("onhome") => self.smart_of(wid).is_some_and(|s| !s.items().is_empty()),
            _ => false,
        }
    }

    /// The type of the card `wid`.
    fn kind_of(&self, wid: &str) -> String {
        self.cards().iter().find(|e| e.id == wid).map(|e| e.kind.clone()).unwrap_or_default()
    }

    /// The smart-home tiles, as the helper lists them.
    pub fn onhome_tiles(&self) -> Vec<Value> {
        super::onhome::tiles(&self.onhome)
    }

    /// How many tiles the helper lists: the report counted them by copying all of them, 75 us of the host's thread each time it was built.
    pub fn onhome_tile_count(&self) -> usize {
        super::onhome::tile_count(&self.onhome)
    }

    /// What the card says of the last control that was refused, while it is said.
    pub fn onhome_notice(&self) -> Option<&str> {
        self.onhome_said.as_ref().map(|(w, _)| w.as_str())
    }

    /// The id of the thing whose sheet is open, whatever kind of sheet it is.
    fn sheet_id(&self) -> Option<&str> {
        self.sheet
            .as_ref()
            .map(|s| s.id.as_str())
            .or_else(|| self.history.as_ref().map(|h| h.id.as_str()))
            .or_else(|| self.colour.as_ref().map(|c| c.id.as_str()))
            .or_else(|| self.thermostat.as_ref().map(|t| t.id.as_str()))
    }

    /// The tile the open sheet is for, as the topic has it now.
    pub fn sheet_tile(&self) -> Option<Value> {
        let id = self.sheet_id()?;
        self.onhome_tiles().into_iter().find(|t| t.get("id").and_then(Value::as_str) == Some(id))
    }

    /// Whether a turn of the wheel does something on Home now: a smart-home card has more than it shows.
    pub fn wheel_scrolls(&self) -> bool {
        self.active && self.layer == Layer::None && self.smarts.iter().any(|(_, s)| s.plan().max_offset() > 0)
    }

    /// Whether the keys of the map page that Home has to ask the compositor for by name are in use: the wheel zooms the map and the channel keys step
    /// its loop.
    pub fn radar_takes_keys(&self) -> bool {
        self.active && self.layer == Layer::Weather && self.wview.page == super::weather_view::Page::Radar
    }

    /// A notch of the wheel: the smart-home card under the pointer scrolls a stride, the other way back; on the map it zooms, about the pointer when the
    /// pointer is shown and about the middle of the map when not (a wheel turned towards the viewer zooms out).
    fn wheel(&mut self, down: bool, out: &mut Vec<Effect>) {
        if self.layer == Layer::Weather {
            if self.radar_takes_keys() && self.radar.open {
                let anchor = self.cursor_visible.then_some(self.pointer_at);
                let by = if down { -WHEEL_STEP } else { WHEEL_STEP };
                out.extend(self.radar.zoom_by(by, anchor, self.now));
                self.redraw(out);
            }
            return;
        }
        if self.layer != Layer::None {
            return;
        }
        let (x, y) = self.pointer_at;
        let by = if down { 1 } else { -1 };
        if let Some((_, s)) = self.smarts.iter_mut().find(|(_, s)| s.contains(x, y))
            && s.scroll(by)
        {
            self.redraw(out);
        }
    }

    fn send_act(&mut self, act: &super::onhome::Act, out: &mut Vec<Effect>) {
        out.push(Effect::Request { tag: ONHOME_TAG, method: "POST".into(), path: "/v1/onhome/act".into(), body: Some(json!({"id": act.id, "verb": act.verb, "value": act.value})) });
    }

    /// How long OK is held for a hold: the cue on an item that has one runs as long.
    pub fn hold_ms(&self) -> u64 {
        self.hold_ms
    }

    // ---- the smart-home card

    /// The state of the smart-home card `wid`.
    pub fn smart_of(&self, wid: &str) -> Option<&Smart> {
        self.smarts.iter().find(|(id, _)| id == wid).map(|(_, s)| s)
    }

    fn smart_mut(&mut self, wid: &str) -> Option<&mut Smart> {
        self.smarts.iter_mut().find(|(id, _)| id == wid).map(|(_, s)| s)
    }

    /// The card with the inner focus, when it is a smart-home card.
    fn onhome_inner(&self) -> Option<String> {
        self.inner.clone().filter(|w| self.kind_of(w) == "onhome")
    }

    /// Whether a held OK is something: on an item with nothing under a hold it is a press, made when OK is released.
    fn hold_applies(&self) -> bool {
        match self.onhome_inner() {
            Some(wid) if self.layer == Layer::None => self.smart_of(&wid).is_some_and(Smart::can_hold),
            _ => true,
        }
    }

    /// The item of a smart-home card under the pointer, with its card.
    fn smart_hit(&self, x: f32, y: f32) -> Option<(String, usize)> {
        self.smarts.iter().find_map(|(wid, s)| s.hit(x, y).map(|i| (wid.clone(), i)))
    }

    /// The pointer went down on the item `i` of the card `wid`: the card is entered with it in front and pressed.
    fn press_item(&mut self, wid: &str, i: usize) {
        let now = self.now;
        if let Some(id) = self.widget_id(wid) {
            self.focus.focus(id);
        }
        if let Some(s) = self.smart_mut(wid) {
            s.press_item(i, now);
        }
        self.inner = Some(wid.to_owned());
        self.item_press = Some((wid.to_owned(), i, now, false));
    }

    /// The smart-home cards follow the layout: one state for each, in its box.
    fn sync_smarts(&mut self) {
        let boxes: Vec<(String, ui::geometry::Frame)> = self
            .cards()
            .into_iter()
            .filter(|e| e.kind == "onhome")
            .map(|e| {
                let r = self.rect(&e);
                (e.id.clone(), ui::geometry::Frame::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32))
            })
            .collect();
        self.smarts.retain(|(id, _)| boxes.iter().any(|(b, _)| b == id));
        let metrics = self.onhome_metrics;
        for (id, frame) in boxes {
            if self.smart_of(&id).is_none() {
                let mut s = Smart::new();
                s.set_topics(&self.onhome, &self.cameras);
                self.smarts.push((id.clone(), s));
            }
            if let Some(s) = self.smart_mut(&id) {
                s.set_geometry(Some(frame), metrics);
            }
        }
    }

    /// The topics changed: every smart-home card makes its items again.
    fn feed_smarts(&mut self) {
        let (onhome, cameras) = (self.onhome.clone(), self.cameras.clone());
        for (_, s) in &mut self.smarts {
            s.set_topics(&onhome, &cameras);
        }
    }

    /// The surface has measured the card's type: the items are laid out by those numbers.
    pub fn set_onhome_metrics(&mut self, m: Metrics) {
        if m != self.onhome_metrics {
            self.onhome_metrics = m;
            self.sync_smarts();
            self.dirty = true;
        }
    }

    /// A key on the entered smart-home card: its items have it, until Back or a key they do not have.
    fn smart_key(&mut self, wid: &str, name: &str, out: &mut Vec<Effect>) {
        let now = self.now;
        let done = self.smart_mut(wid).and_then(|s| s.key(name, now));
        match done {
            None => self.leave_card(wid),
            Some(o) => self.smart_out(wid, o, out),
        }
    }

    /// The card is left: nothing on it is pressed and no menu is open.
    fn leave_card(&mut self, wid: &str) {
        if let Some(s) = self.smart_mut(wid) {
            s.leave();
        }
        self.inner = None;
        self.inner_index = None;
    }

    fn smart_hold(&mut self, out: &mut Vec<Effect>) {
        if let Some(wid) = self.onhome_inner() {
            self.smart_hold_of(&wid, out);
        }
    }

    fn smart_hold_of(&mut self, wid: &str, out: &mut Vec<Effect>) {
        let done = self.smart_mut(wid).map_or(SmartOut::Nothing, Smart::hold);
        self.smart_out(wid, done, out);
        self.redraw(out);
    }

    /// What the card asks for: a control to send, a sheet, the camera viewer, a change of the owner's list.
    fn smart_out(&mut self, wid: &str, done: SmartOut, out: &mut Vec<Effect>) {
        match done {
            SmartOut::Nothing => {}
            SmartOut::Leave => self.leave_card(wid),
            SmartOut::Act { seq, act } => {
                self.act_tag = (self.act_tag + 1) % 0x1_0000;
                let tag = ACT_TAG + self.act_tag;
                self.acts.push((tag, wid.to_owned(), seq));
                out.push(Effect::Request { tag, method: "POST".into(), path: "/v1/onhome/act".into(), body: Some(json!({"id": act.id, "verb": act.verb, "value": act.value})) });
            }
            SmartOut::Controls(id) => self.open_controls(&id, false, out),
            SmartOut::Said(words) => self.onhome_said = Some((words, self.now + 4000)),
            SmartOut::Unlock(id) => self.open_controls(&id, true, out),
            SmartOut::History(id) => self.open_history(&id, out),
            SmartOut::Colour(id) => self.open_colour(&id, out),
            SmartOut::Camera(id) => self.requests.push(Request::CameraFull(id)),
            SmartOut::Pick(body) => out.push(Effect::Request { tag: PICKS_TAG, method: "POST".into(), path: "/v1/onhome/picks".into(), body: Some(body) }),
        }
        self.redraw(out);
    }

    /// The controls of the tile `id` over Home; with `asking` the question "Unlock <name>?" with Cancel in front.
    fn open_controls(&mut self, id: &str, asking: bool, out: &mut Vec<Effect>) {
        // a thermostat has a sheet of its own: its room, its set point and its modes
        let tile = self.onhome_tiles().into_iter().find(|t| t.get("id").and_then(Value::as_str) == Some(id));
        if !asking && let Some(t) = tile.filter(|t| t.get("group").and_then(Value::as_str) == Some("climate") && t.get("available").and_then(Value::as_bool) != Some(false)) {
            self.thermostat = Some(super::onhome::thermostat::ThermostatSheet::new(id, &t));
        } else {
            self.sheet = Some(if asking { super::onhome::Sheet::asking(id) } else { super::onhome::Sheet::new(id) });
        }
        self.open_layer(Layer::OnHome, SCOPE_ONHOME);
        self.redraw(out);
    }

    /// The history of the sensor `id` over Home: the sheet opens at once and says it is loading, and the helper's answer fills it.
    fn open_history(&mut self, id: &str, out: &mut Vec<Effect>) {
        let sheet = super::onhome::sensor::SensorSheet::new(id);
        out.push(Effect::Request { tag: HISTORY_TAG, method: "GET".into(), path: sheet.path(), body: None });
        self.history = Some(sheet);
        self.open_layer(Layer::OnHome, SCOPE_ONHOME);
        self.redraw(out);
    }

    /// The colours of the lamp `id` over Home, from the tile as the topic has it; a lamp with nothing to set but on and off has its controls.
    fn open_colour(&mut self, id: &str, out: &mut Vec<Effect>) {
        let tile = self.onhome_tiles().into_iter().find(|t| t.get("id").and_then(Value::as_str) == Some(id));
        match tile.map(|t| super::onhome::colour::ColourSheet::new(id, &t)).filter(|s| !s.rows.is_empty()) {
            Some(sheet) => {
                self.colour = Some(sheet);
                self.open_layer(Layer::OnHome, SCOPE_ONHOME);
                self.redraw(out);
            }
            None => self.open_controls(id, false, out),
        }
    }

    /// The answer to a control an item sent: it was done, or it was refused or failed and the item goes back to what the topic says, in words.
    fn act_answered(&mut self, tag: u64, status: u16, data: &Value, out: &mut Vec<Effect>) {
        let Some(at) = self.acts.iter().position(|(t, ..)| *t == tag) else { return };
        let (_, wid, seq) = self.acts.remove(at);
        let words = (status != 200).then(|| data.pointer("/error/message").and_then(Value::as_str).filter(|w| !w.is_empty()).unwrap_or("Home Assistant did not do that").to_owned());
        let now = self.now;
        if let Some(s) = self.smart_mut(&wid) {
            s.answered(seq, words.as_deref(), now);
        }
        if let Some(w) = words {
            self.onhome_said = Some((w, now + 4000));
        }
        self.redraw(out);
    }

    /// Whether the thing the open sheet is for has left Home.
    fn onhome_sheet_gone(&self) -> bool {
        self.sheet_id().is_some() && self.sheet_tile().is_none()
    }

    fn history_key(&mut self, name: &str, out: &mut Vec<Effect>) {
        let Some(h) = self.history.as_mut() else { return };
        match h.key(name) {
            super::onhome::sensor::Done::Close => self.close_layer(),
            super::onhome::sensor::Done::Ask(_) => {
                let path = h.path();
                out.push(Effect::Request { tag: HISTORY_TAG, method: "GET".into(), path, body: None });
            }
            super::onhome::sensor::Done::Nothing => {}
        }
    }

    fn colour_key(&mut self, name: &str, out: &mut Vec<Effect>) {
        let now = self.now;
        let Some(c) = self.colour.as_mut() else { return };
        if c.key(name, now) == super::onhome::colour::Done::Close {
            self.close_layer();
            return;
        }
        self.send_colour(out);
    }

    /// The calls of the colour sheet that are due: what was changed since the last, a quarter of a second apart at most.
    fn send_colour(&mut self, out: &mut Vec<Effect>) {
        let now = self.now;
        let due = self.colour.as_mut().map(|c| c.tick(now)).unwrap_or_default();
        for act in due {
            self.send_act(&act, out);
        }
    }

    fn thermostat_key(&mut self, name: &str, out: &mut Vec<Effect>) {
        use super::onhome::thermostat::Done;
        let now = self.now;
        let Some(t) = self.thermostat.as_mut() else { return };
        match t.key(name, now) {
            Done::Close => self.close_layer(),
            Done::Send(act) => self.send_act(&act, out),
            Done::Nothing => {}
        }
    }

    /// The call of the thermostat sheet that is due: a set point that has rested.
    fn send_thermostat(&mut self, out: &mut Vec<Effect>) {
        let now = self.now;
        if let Some(act) = self.thermostat.as_mut().and_then(|t| t.due(now)) {
            self.send_act(&act, out);
        }
    }

    /// A click on an open thermostat sheet: a step of a set point, or a mode; a click beside the sheet puts it away.
    fn thermostat_pressed(&mut self, x: f32, y: f32, out: &mut Vec<Effect>) {
        use super::onhome::thermostat::{Done, Row, ThermostatSheet};
        use super::onhome::thermostat_surface::{Hit, hit, layout};
        let now = self.now;
        let m = self.onhome_metrics;
        let Some(t) = self.thermostat.as_mut() else { return };
        let chars = t.modes.iter().map(|m| ThermostatSheet::mode_word(m).chars().count()).max().unwrap_or(0);
        let lay = layout(&m, &t.shown(), t.modes.len(), chars);
        match hit(&lay, t, x, y) {
            Some(Hit::Minus(row)) | Some(Hit::Plus(row)) if row != Row::Mode => {
                let plus = matches!(hit(&lay, t, x, y), Some(Hit::Plus(_)));
                // the row is in front, and one step is taken as a key would take it
                if let Some(i) = t.rows.iter().position(|r| *r == row) {
                    t.at = i;
                }
                t.key(if plus { "right" } else { "left" }, now);
            }
            Some(Hit::Chip(i)) => {
                if let Done::Send(act) = t.choose(i, now) {
                    self.send_act(&act, out);
                }
            }
            None if !lay.pane.contains(x, y) => self.close_layer(),
            _ => {}
        }
    }

    /// A click on an open history or colour sheet: a chip of the range, the ring, a track, a swatch or a chip of the effects; a click beside the
    /// sheet puts it away.
    fn sheet_pressed(&mut self, x: f32, y: f32, out: &mut Vec<Effect>) {
        if self.thermostat.is_some() {
            return self.thermostat_pressed(x, y, out);
        }
        use super::onhome::{colour, colour_surface, sensor_surface};
        let (m, now) = (self.onhome_metrics, self.now);
        if self.history.is_some() {
            let shape = self.sheet_tile().map_or(sensor_surface::Shape::Number, |t| sensor_surface::shape_of(&t));
            let lay = sensor_surface::layout(&m, shape);
            let chip = lay.chips.iter().position(|c| ui::geometry::Frame::new(lay.pane.x + c.x, lay.pane.y + c.y, c.w, c.h).contains(x, y));
            match chip {
                Some(i) => {
                    if let Some(h) = self.history.as_mut()
                        && let super::onhome::sensor::Done::Ask(_) = h.pick(i)
                    {
                        out.push(Effect::Request { tag: HISTORY_TAG, method: "GET".into(), path: h.path(), body: None });
                    }
                }
                None if !lay.pane.contains(x, y) => self.close_layer(),
                None => {}
            }
            return;
        }
        let Some(c) = self.colour.as_mut() else { return };
        let lay = colour_surface::layout(&m, &c.rows, c.effects.len());
        let shown = lay.rows.iter().find(|r| r.row == colour::Row::Effect).map_or(0, |r| r.cells.len());
        let window = colour_surface::chip_window(c.effect, c.effects.len(), shown);
        match colour_surface::hit(&lay, window, x, y) {
            Some(colour_surface::Hit::Ring(hue)) => {
                c.set_row(colour::Row::Hue, hue / 360.0, now);
                self.colour_held = Some(Held::Ring);
            }
            Some(colour_surface::Hit::Track(row, t)) => {
                c.set_row(row, t, now);
                self.colour_held = Some(Held::Track(row));
            }
            Some(colour_surface::Hit::Swatch(i)) => c.pick(colour::Row::Preset, i, now),
            Some(colour_surface::Hit::Chip(i)) => c.pick(colour::Row::Effect, i, now),
            None if !lay.pane.contains(x, y) => {
                self.close_layer();
                return;
            }
            None => {}
        }
        self.send_colour(out);
    }

    /// The pointer moves with its button down on the ring or a track: the thumb follows it, wherever it goes.
    fn colour_dragged(&mut self, x: f32, y: f32, out: &mut Vec<Effect>) {
        use super::onhome::{colour, colour_surface};
        let (m, now) = (self.onhome_metrics, self.now);
        let (Some(held), Some(c)) = (self.colour_held, self.colour.as_mut()) else { return };
        let lay = colour_surface::layout(&m, &c.rows, c.effects.len());
        match held {
            Held::Ring => {
                if let Some(hue) = colour_surface::round(&lay, x, y) {
                    c.set_row(colour::Row::Hue, hue / 360.0, now);
                }
            }
            Held::Track(row) => c.set_row(row, colour_surface::along(&lay, row, x), now),
        }
        self.send_colour(out);
        self.redraw(out);
    }

    fn sheet_key(&mut self, name: &str, out: &mut Vec<Effect>) {
        if self.thermostat.is_some() {
            return self.thermostat_key(name, out);
        }
        if self.history.is_some() {
            return self.history_key(name, out);
        }
        if self.colour.is_some() {
            return self.colour_key(name, out);
        }
        let Some(tile) = self.sheet_tile() else {
            self.close_layer();
            return;
        };
        let now = self.now;
        let done = self.sheet.as_mut().map_or(super::onhome::Done::Close, |s| s.key(name, &tile, now));
        match done {
            super::onhome::Done::Close => self.close_layer(),
            super::onhome::Done::Send(act) => self.send_act(&act, out),
            super::onhome::Done::Unlock(id) => {
                // the key was the person's OK on the answer; the token is read now and only then is the unlock asked
                self.unlock_of = Some(id);
                out.push(Effect::Read { tag: UNLOCK_TAG, path: "onhome/shell.json".into() });
                self.close_layer();
            }
            super::onhome::Done::Nothing => {}
        }
    }

    /// A key inside a card: the content's rows. Returns false when the content gives the keys back.
    fn inner_key(&mut self, wid: &str, name: &str, out: &mut Vec<Effect>) -> bool {
        let kind = self.cards().iter().find(|e| e.id == wid).map(|e| e.kind.clone()).unwrap_or_default();
        let n = match kind.as_str() {
            "homeassistant" => self.ha_items().len(),
            "news" => self.news_items().len(),
            _ => 0,
        };
        if n == 0 {
            return false;
        }
        let i = self.inner_index.unwrap_or(0);
        let cols = 1;
        match name {
            "left" => self.inner_index = Some(i.saturating_sub(1)),
            "right" => self.inner_index = Some((i + 1).min(n - 1)),
            "up" => self.inner_index = Some(if i >= cols { i - cols } else { i }),
            "down" => self.inner_index = Some(if i + cols < n { i + cols } else { i }),
            "ok" => {
                if kind == "homeassistant"
                    && let Some(card) = self.ha_items().get(i)
                    && let Some(a) = card.pointer("/card/actions/0/action").and_then(Value::as_str)
                {
                    let plugin = card.get("pluginId").and_then(Value::as_str).unwrap_or(hub::HA).to_owned();
                    let target = card.pointer("/card/actions/0/target").and_then(Value::as_str);
                    out.push(Effect::Post { path: format!("/v1/plugins/{plugin}/actions/{a}"), body: target.map_or_else(|| json!({}), |t| json!({"target": t})) });
                }
                if kind == "news"
                    && let Some(item) = self.news_items().get(i)
                {
                    self.open_story(wid, item, out);
                }
            }
            _ => {}
        }
        true
    }

    /// A story of the news card opens in the mini frame, which grows out of the card (`NewsWidget.qml`'s `openMini`).
    fn open_story(&mut self, wid: &str, item: &Value, out: &mut Vec<Effect>) {
        let text = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
        let link = text("link");
        if link.is_empty() {
            return;
        }
        let Some(entry) = self.cards().into_iter().find(|e| e.id == wid) else { return };
        let from = self.rect(&entry);
        let area = grid::area(&self.drawn_layout());
        // 1520 by 860, or what the room under the bar leaves at a large text size (the bar grows with it)
        let (w, h) = (1520.0, 860.0f32.min(area.h as f32));
        let to = [((1920.0 - w) / 2.0f32).round(), (area.y as f32 + (area.h as f32 - h) / 2.0).round(), w, h];
        let ago = item.get("pubMs").and_then(Value::as_u64).map(|p| super::widgets::ago(self.now.saturating_sub(p))).unwrap_or_default();
        let source = text("src");
        let subtitle = if ago.is_empty() { source.clone() } else { format!("{source} \u{b7} {ago}") };
        self.notes.push(Note::Frame { page: Page { url: link, title: text("title"), subtitle, source, ago }, from: [from.x as f32, from.y as f32, from.w as f32, from.h as f32], to });
        self.redraw(out);
    }

    fn activate_card(&mut self, wid: &str, out: &mut Vec<Effect>) {
        let kind = self.cards().iter().find(|e| e.id == wid).map(|e| e.kind.clone()).unwrap_or_default();
        if kind == "homeassistant" {
            self.requests.push(Request::SettingsPage("homeassistant".into()));
        }
        if kind == "onhome" {
            self.requests.push(Request::SettingsPage("onhome".into()));
        }
        if kind == "cameras" {
            self.requests.push(Request::Cameras);
        }
        if kind == "weather" {
            self.wview.open();
            self.open_layer(Layer::Weather, SCOPE_WEATHER);
            self.redraw(out);
        }
        out.push(Effect::Log(format!("card {wid} ({kind}) activated")));
    }

    /// The chips of the At-home card: every connected plugin's cards but those Home draws in full elsewhere, the most urgent first.
    pub fn ha_items(&self) -> Vec<Value> {
        hub::home_cards(&self.plugins)
    }

    /// What the At-home card says when there is no chip: Home Assistant's own state, from the same roster and lease as Settings' page.
    pub fn ha_lane(&self) -> hub::Lane {
        hub::lane_none(self.plugins.get("plugins").unwrap_or(&Value::Null), self.plugins.get("ha.link").unwrap_or(&Value::Null))
    }

    pub fn news_items(&self) -> Vec<Value> {
        self.news.pointer("/doc/items").and_then(Value::as_array).cloned().unwrap_or_default()
    }

    pub fn camera_items(&self) -> Vec<Value> {
        self.cameras.get("cameras").and_then(Value::as_array).cloned().unwrap_or_default()
    }

    // ---- the layers over Home
    fn open_layer(&mut self, layer: Layer, scope: Scope) {
        if self.layer != Layer::None {
            self.stack.push(self.layer);
        }
        self.layer = layer;
        self.focus.open_modal(scope);
    }

    fn close_layer(&mut self) {
        if self.layer == Layer::None {
            return;
        }
        if self.layer == Layer::OnHome {
            self.sheet = None;
            self.history = None;
            self.colour = None;
            self.colour_held = None;
            self.thermostat = None;
        }
        if self.layer == Layer::Popover
            && let Some(p) = self.popover.take()
        {
            self.keep_popover(p);
            if std::mem::take(&mut self.wallpaper_return_settings) {
                self.requests.push(Request::SettingsPage("look".into()));
            }
            self.popover_lay = None;
            for i in 0..popover::ROWS.len() as u32 {
                self.focus.unregister(Id(popover::ROW_ID + i));
            }
        }
        if self.layer == Layer::Weather && self.radar.open {
            let fx = self.radar.close();
            self.deferred.extend(fx);
        }
        self.focus.close_modal();
        self.layer = self.stack.pop().unwrap_or(Layer::None);
    }

    pub fn close_all(&mut self) {
        self.wallpaper_return_settings = false;
        while self.layer != Layer::None {
            if self.layer == Layer::Edit {
                if let Some(ed) = self.editor.take() {
                    self.preview_layout(ed.orig.clone());
                }
            }
            self.close_layer();
        }
        self.inner = None;
        self.inner_index = None;
        self.menu_app = None;
        self.confirm = None;
    }

    pub fn open_apps(&mut self, out: &mut Vec<Effect>) {
        self.apps.open();
        self.open_layer(Layer::Apps, SCOPE_APPS);
        self.apps.register(&mut self.focus);
        self.redraw(out);
    }

    pub fn open_search(&mut self, out: &mut Vec<Effect>) {
        let apps: Vec<Value> = self.apps.all().iter().map(|a| json!({"id": a.id, "title": a.title, "iconUrl": a.icon})).collect();
        self.search.configure(&self.theme, self.text_scale);
        self.search.open(apps, self.now);
        self.search.set_pages(self.search_pages.clone());
        self.pages_wanted = true;
        self.open_layer(Layer::Search, SCOPE_SEARCH);
        self.search.register(&mut self.focus);
        self.redraw(out);
    }

    pub fn open_picker(&mut self, out: &mut Vec<Effect>) {
        if self.profiles.get("ready").and_then(Value::as_bool) != Some(true) {
            return;
        }
        self.picker.open(self.now);
        self.open_layer(Layer::Picker, SCOPE_PICKER);
        self.picker.register(&mut self.focus);
        self.redraw(out);
    }

    /// The layout editor on `id` (a hold of OK on a card, Settings' "Edit on Home").
    pub fn edit_home(&mut self, id: &str, out: &mut Vec<Effect>) {
        if self.layer == Layer::Edit {
            return;
        }
        let ed = Editor::new(&self.layout, id, self.now, self.text_scale);
        self.preview_layout(ed.work.clone());
        ed.register(&mut self.focus, SCOPE_EDIT);
        self.editor = Some(ed);
        self.open_layer(Layer::Edit, SCOPE_EDIT);
        if let Some(ed) = &mut self.editor {
            ed.register(&mut self.focus, SCOPE_EDIT);
        }
        self.redraw(out);
    }

    fn edit_end(&mut self, out: &mut Vec<Effect>) {
        let Some(ed) = self.editor.take() else { return };
        self.close_layer();
        ed.unregister(&mut self.focus);
        // no item menu is left open on a card the editor was arranging
        for (_, s) in &mut self.smarts {
            s.leave();
        }
        if let Some(first) = self.cards().first().map(|e| e.id.clone())
            && let Some(id) = self.widget_id(&ed.sel).or_else(|| self.widget_id(&first))
        {
            self.focus.focus(id);
        }
        self.redraw(out);
    }

    /// Whether the layout editor is arranging the items of the smart-home card `wid`: the card is entered and its items take the keys.
    pub fn items_editing(&self, wid: &str) -> bool {
        self.layer == Layer::Edit && self.editor.as_ref().is_some_and(|e| e.zone == super::editor::Zone::Items && e.sel == wid)
    }

    /// A key while the layout editor arranges the items of its chosen card: the arrows choose an item, OK (or Menu) opens its menu (size, place,
    /// hide) and then the menu has the keys, Back closes the menu and then leaves the items for the bar. OK never switches the item: this is not
    /// the card's own use of it.
    fn items_key(&mut self, name: &str, out: &mut Vec<Effect>) {
        let Some(wid) = self.editor.as_ref().map(|e| e.sel.clone()) else { return };
        let now = self.now;
        let arranging = self.smart_of(&wid).is_some_and(|s| s.arrange.is_some());
        let Some(s) = self.smart_mut(&wid) else {
            self.items_done();
            return;
        };
        if !arranging {
            match name {
                "back" => {
                    s.leave();
                    self.items_done();
                    return;
                }
                "ok" | "menu" => {
                    s.open_arrange();
                    return;
                }
                _ => {}
            }
        }
        if let Some(done) = s.key(name, now) {
            self.smart_out(&wid, done, out);
        }
    }

    /// The items are done: the bar has the keys again, on the button that brought the person here.
    fn items_done(&mut self) {
        if let Some(ed) = &mut self.editor {
            ed.zone = super::editor::Zone::Bar;
            ed.register(&mut self.focus, SCOPE_EDIT);
        }
    }

    fn edit_key(&mut self, name: &str, dir: Option<Dir>, out: &mut Vec<Effect>) {
        if self.editor.as_ref().is_some_and(|e| e.zone == super::editor::Zone::Items) {
            self.items_key(name, out);
            return;
        }
        let Some(ed) = &mut self.editor else { return };
        let outs = ed.key(name, dir, &mut self.focus, self.now);
        self.edit_outs(outs, out);
        if let Some(ed) = &mut self.editor {
            ed.register(&mut self.focus, SCOPE_EDIT);
        }
    }

    fn apps_key(&mut self, name: &str, dir: Option<Dir>, out: &mut Vec<Effect>) {
        let action = self.apps.key(name, dir, &mut self.focus);
        self.apps_out(action, out);
    }

    fn apps_out(&mut self, action: super::apps::AppsOut, out: &mut Vec<Effect>) {
        match action {
            super::apps::AppsOut::Nothing => {}
            super::apps::AppsOut::Close => {
                self.apps.unregister(&mut self.focus);
                self.close_layer();
            }
            super::apps::AppsOut::Launch(app) => self.launch(&app.id, &app.params, &app.title, out),
            super::apps::AppsOut::Search => self.open_search(out),
            super::apps::AppsOut::Menu(app) => self.open_menu(app, "library", out),
        }
    }

    fn search_key(&mut self, name: &str, _dir: Option<Dir>, out: &mut Vec<Effect>) {
        let o = self.search.input(name, true, false, &mut self.focus, self.now);
        self.search_out(o, out);
    }

    /// What the search sheet ended with: a pick, a name entered, or its closing.
    fn search_out(&mut self, o: SearchOut, out: &mut Vec<Effect>) {
        match o {
            SearchOut::Nothing => {}
            SearchOut::Close => {
                self.search.unregister(&mut self.focus);
                self.close_layer();
                if self.layer == Layer::Apps {
                    self.apps.register(&mut self.focus);
                }
            }
            SearchOut::Picked { kind, id, text } => {
                self.search.unregister(&mut self.focus);
                self.close_layer();
                match kind.as_str() {
                    "app" => {
                        if let Some(a) = self.apps.by_id(&id).cloned() {
                            self.launch(&a.id, &a.params, &a.title, out);
                        }
                    }
                    "web" => self.start_app("com.webos.app.browser", &json!({"target": format!("https://duckduckgo.com/?q={}", text.replace(' ', "+"))}), "Browser", out),
                    // a page of Settings: Settings opens at it (the shell's kind of that name when it is one)
                    "page" => self.requests.push(Request::SettingsPage(id)),
                    _ => out.push(Effect::Log(format!("search picked {kind}"))),
                }
            }
            SearchOut::Entered(text) => {
                self.search.unregister(&mut self.focus);
                self.close_layer();
                if let Some(app) = self.menu_app.take()
                    && let Some(id) = app.get("id").and_then(Value::as_str)
                {
                    self.keep_apps(json!({"names": {id: text}}), out);
                }
            }
        }
    }

    fn picker_key(&mut self, name: &str, dir: Option<Dir>, out: &mut Vec<Effect>) {
        match self.picker.key(name, dir, &mut self.focus, self.now) {
            PickerOut::Nothing => {}
            PickerOut::Close => {
                self.picker.unregister(&mut self.focus);
                self.close_layer();
            }
            PickerOut::Switch { id, pin } => {
                let mut body = json!({"id": id});
                if let Some(p) = pin {
                    body["pin"] = json!(p);
                }
                out.push(Effect::Request { tag: SWITCH_TAG, method: "POST".into(), path: "/v1/profiles/switch".into(), body: Some(body) });
                out.push(Effect::Log(format!("profiles: switch to {id} asked")));
            }
        }
    }

    // ---- the status bar's settings popover
    /// Opens the popover under status item `item`: the keys and the pointer are its until Back (or a click outside).
    fn open_popover(&mut self, item: usize, out: &mut Vec<Effect>) {
        self.popover = Some(Popover::new(item));
        self.open_layer(Layer::Popover, popover::SCOPE);
        self.register_popover();
        self.redraw(out);
    }

    pub fn edit_wallpaper(&mut self, out: &mut Vec<Effect>) {
        let item = self.status_index("settings").unwrap_or(0);
        self.open_popover(item, out);
        self.wallpaper_return_settings = true;
    }

    /// Where the panel and its rows are now (the surface draws what this says), and the rows in view registered as the pointer's targets.
    pub fn popover_layout(&mut self) -> Option<popover::Layout> {
        let item = self.popover.as_ref()?.item;
        let (sx, sy) = self.safe();
        let (x, y, w, h) = self.status_frames().get(item).copied().unwrap_or((sx, sy, 48.0, 48.0));
        let (bar, scale) = (self.status_row(), self.text_scale);
        self.popover.as_mut().map(|p| p.layout(bar, ui::geometry::Frame::new(x, y, w, h), scale))
    }

    fn register_popover(&mut self) {
        self.popover_lay = self.popover_layout();
        let Some(layout) = self.popover_lay.clone() else { return };
        for i in 0..popover::ROWS.len() as u32 {
            match layout.rows.iter().find(|(r, _)| *r as u32 == i) {
                Some((_, f)) => self.focus.register(Id(popover::ROW_ID + i), *f, popover::REGION, popover::SCOPE),
                None => self.focus.unregister(Id(popover::ROW_ID + i)),
            }
        }
        if let Some(p) = &self.popover {
            self.focus.focus(Id(popover::ROW_ID + p.index as u32));
        }
    }

    /// A row did something: a value changed (the theme has it now, so Home behind shows it), or an action goes.
    fn popover_done(&mut self, done: Done, out: &mut Vec<Effect>) {
        match done {
            Done::Nothing => {}
            Done::Changed(key, value) => {
                if !self.theme.get("wallpaperMotion").is_some_and(Value::is_object) {
                    self.theme["wallpaperMotion"] = json!({});
                }
                self.theme["wallpaperMotion"][key] = value;
            }
            Done::TvSettings => {
                self.wallpaper_return_settings = false;
                self.close_layer();
                self.start_app("com.palm.app.settings", &json!({}), "Settings", out);
            }
        }
    }

    /// Whether the radar's loop plays by itself: its page is up and seen, and motion is not reduced.
    pub fn radar_loops(&self) -> bool {
        self.weather_view_runs() && self.wview.page == super::weather_view::Page::Radar && !self.motion_reduced
    }

    fn weather_key(&mut self, name: &str, dir: Option<Dir>, out: &mut Vec<Effect>) {
        // Left and Right are the view's pages on every page; on the map the rest of the remote is the map's: Up and Down zoom, OK plays and holds the
        // loop, the channel keys step it
        if self.wview.page == super::weather_view::Page::Radar {
            let zoom = match dir {
                Some(Dir::Up) => Some(KEY_STEP),
                Some(Dir::Down) => Some(-KEY_STEP),
                _ => None,
            };
            if let Some(d) = zoom {
                out.extend(self.radar.zoom_by(d, None, self.now));
                return;
            }
            if matches!(name, "ok" | "chup" | "chdown") {
                let (fx, _) = self.radar.key(name, self.now);
                out.extend(fx);
                return;
            }
        }
        let step = match dir {
            Some(Dir::Left) => -1,
            Some(Dir::Right) => 1,
            _ => 0,
        };
        if self.wview.key(name, step) == super::weather_view::Done::Close {
            self.close_layer();
        }
        self.sync_radar(out);
    }

    /// The helper's radar runs while the radar page is the one shown, and not otherwise.
    fn sync_radar(&mut self, out: &mut Vec<Effect>) {
        let on = self.layer == Layer::Weather && self.wview.page == super::weather_view::Page::Radar;
        if on && !self.radar.open {
            let (lat, lon) = (self.weather.pointer("/doc/lat").and_then(Value::as_f64), self.weather.pointer("/doc/lon").and_then(Value::as_f64));
            if let (Some(lat), Some(lon)) = (lat, lon) {
                out.extend(self.radar.open(lat, lon, !self.light, self.now));
            }
        } else if !on && self.radar.open {
            out.extend(self.radar.close());
        }
    }

    pub fn set_motion_reduced(&mut self, reduced: bool) {
        self.motion_reduced = reduced;
    }

    fn popover_key(&mut self, name: &str, dir: Option<Dir>, out: &mut Vec<Effect>) {
        let wm = self.theme.get("wallpaperMotion").cloned().unwrap_or(Value::Null);
        let done = match (name, dir) {
            ("back", _) => {
                self.close_layer();
                return;
            }
            (_, Some(Dir::Up | Dir::Down)) => {
                let d = if dir == Some(Dir::Up) { -1 } else { 1 };
                if let Some(p) = self.popover.as_mut() {
                    p.moved(d);
                }
                Done::Nothing
            }
            (_, Some(Dir::Left)) => self.popover.as_mut().map_or(Done::Nothing, |p| p.adjust(-1, &wm)),
            (_, Some(Dir::Right)) => self.popover.as_mut().map_or(Done::Nothing, |p| p.adjust(1, &wm)),
            ("ok", _) => self.popover.as_mut().map_or(Done::Nothing, |p| p.press(&wm)),
            _ => Done::Nothing,
        };
        self.popover_done(done, out);
        if self.layer == Layer::Popover {
            self.register_popover();
        }
    }

    /// A click on a row of the popover: a slider takes a step up, a toggle turns, an action goes (OK on it).
    fn popover_click(&mut self, id: Id, out: &mut Vec<Effect>) {
        let Some(i) = id.0.checked_sub(popover::ROW_ID).filter(|i| (*i as usize) < popover::ROWS.len()).map(|i| i as usize) else { return };
        let wm = self.theme.get("wallpaperMotion").cloned().unwrap_or(Value::Null);
        let done = self.popover.as_mut().map_or(Done::Nothing, |p| {
            p.index = i;
            if matches!(p.def().kind, popover::Kind::Slider { .. }) { p.adjust(1, &wm) } else { p.press(&wm) }
        });
        self.popover_done(done, out);
        if self.layer == Layer::Popover {
            self.register_popover();
        }
    }

    /// The popover has closed: what it changed goes to the theme at the next update.
    fn keep_popover(&mut self, mut p: Popover) {
        if let Some(patch) = p.take_dirty() {
            match &mut self.persist {
                Some(Value::Object(old)) => old.extend(patch.as_object().cloned().unwrap_or_default()),
                other => *other = Some(patch),
            }
        }
    }

    // ---- the tile menu
    fn open_tile_menu(&mut self, out: &mut Vec<Effect>) {
        if let Some(p) = self.dock.focused().cloned() {
            let app = json!({"id": p.id, "title": p.title, "iconUrl": p.icon, "params": p.params, "onDock": true, "unmovable": p.unmovable, "removable": p.removable});
            self.open_menu(app, "dock", out);
        }
    }

    pub fn menu_rows(app: &Value, source: &str) -> Vec<MenuRow> {
        let on_dock = app.get("onDock").and_then(Value::as_bool).unwrap_or(false);
        let unmovable = app.get("unmovable").and_then(Value::as_bool).unwrap_or(false);
        let hidden = app.get("hidden").and_then(Value::as_bool).unwrap_or(false);
        let mut rows = vec![MenuRow { id: "open", label: "Open".into(), danger: false }];
        if on_dock && !unmovable {
            rows.push(MenuRow { id: "move", label: "Move on the dock".into(), danger: false });
            rows.push(MenuRow { id: "hide-dock", label: "Hide from the dock".into(), danger: false });
        }
        if source == "library" {
            rows.push(MenuRow { id: if hidden { "show" } else { "hide" }, label: if hidden { "Show".into() } else { "Hide".into() }, danger: false });
        }
        rows.push(MenuRow { id: "rename", label: "Rename".into(), danger: false });
        if app.get("removable").and_then(Value::as_bool) == Some(true) {
            rows.push(MenuRow { id: "uninstall", label: "Uninstall".into(), danger: true });
        }
        rows
    }

    fn open_menu(&mut self, app: Value, source: &str, out: &mut Vec<Effect>) {
        self.menu = Self::menu_rows(&app, source);
        self.menu_index = 0;
        self.menu_app = Some(app);
        self.open_layer(Layer::Menu, SCOPE_MENU);
        self.redraw(out);
    }

    /// The question: Left and Right move between the answers, OK gives the one in front, Back (and Menu) is Keep it. Only Uninstall removes, and
    /// only from this layer.
    fn confirm_key(&mut self, name: &str, dir: Option<Dir>, out: &mut Vec<Effect>) {
        match (name, dir) {
            (_, Some(Dir::Left | Dir::Right)) => {
                let i = usize::from(dir == Some(Dir::Right));
                if let Some(c) = &mut self.confirm {
                    c.index = i;
                }
                self.focus.focus(Id(CONFIRM_ID + i as u32));
            }
            ("ok", _) => {
                let asked = self.confirm.clone();
                self.end_confirm();
                if let Some(c) = asked.filter(|c| c.index == 1) {
                    out.push(Effect::Luna { tag: LAUNCH_TAG, uri: "luna://com.webos.appInstallService/remove".into(), payload: json!({"id": c.app_id}), subscribe: false });
                }
            }
            ("back", _) | ("menu", _) => self.end_confirm(),
            _ => {}
        }
    }

    fn end_confirm(&mut self) {
        self.confirm = None;
        for i in 0..CONFIRM_ANSWERS.len() as u32 {
            self.focus.unregister(Id(CONFIRM_ID + i));
        }
        self.close_layer();
    }

    /// Where the surface put the question's answers: they are focusables of the layer (for the pointer's cue and click), and the one in front has the focus.
    pub fn set_confirm_frames(&mut self, answers: &[ui::geometry::Frame]) {
        let Some(index) = self.confirm.as_ref().map(|c| c.index) else { return };
        for (i, f) in answers.iter().enumerate() {
            self.focus.register(Id(CONFIRM_ID + i as u32), *f, CONFIRM_REGION, SCOPE_CONFIRM);
        }
        if self.focus.current() != Some(Id(CONFIRM_ID + index as u32)) {
            self.focus.focus(Id(CONFIRM_ID + index as u32));
        }
    }

    fn menu_key(&mut self, name: &str, dir: Option<Dir>, out: &mut Vec<Effect>) {
        match (name, dir) {
            (_, Some(Dir::Up)) => self.menu_index = self.menu_index.saturating_sub(1),
            (_, Some(Dir::Down)) => self.menu_index = (self.menu_index + 1).min(self.menu.len().saturating_sub(1)),
            ("back", _) | ("menu", _) => {
                self.close_layer();
                self.menu_app = None;
            }
            ("ok", _) => {
                let id = self.menu.get(self.menu_index).map(|r| r.id).unwrap_or("");
                let app = self.menu_app.clone().unwrap_or(Value::Null);
                let app_id = app.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
                self.close_layer();
                match id {
                    "open" => {
                        let (params, title) = (app.get("params").cloned().unwrap_or(json!({})), app.get("title").and_then(Value::as_str).unwrap_or("").to_owned());
                        self.launch(&app_id, &params, &title, out);
                    }
                    "move" => {
                        if self.layer == Layer::Apps {
                            self.apps.unregister(&mut self.focus);
                            self.close_layer();
                        }
                        if let Some(i) = self.dock.points.iter().position(|p| p.id == app_id) {
                            self.dock.focus(i);
                            self.register_dock();
                            self.focus.focus(Id(DOCK_ID + i as u32));
                            self.dock.begin_move();
                        }
                    }
                    "hide-dock" => {
                        let mut hide: Vec<Value> = self.theme.pointer("/apps/dockHide").and_then(Value::as_array).cloned().unwrap_or_default();
                        hide.push(json!(app_id));
                        self.keep_apps(json!({"dockHide": hide}), out);
                    }
                    "hide" | "show" => {
                        let mut hidden: Vec<Value> = self.theme.pointer("/apps/hidden").and_then(Value::as_array).cloned().unwrap_or_default();
                        hidden.retain(|v| v.as_str() != Some(&app_id));
                        if id == "hide" {
                            hidden.push(json!(app_id));
                        }
                        self.keep_apps(json!({"hidden": hidden}), out);
                    }
                    "rename" => {
                        let title = app.get("title").and_then(Value::as_str).unwrap_or("").to_owned();
                        self.search.configure(&self.theme, self.text_scale);
                        self.search.open_entry(&format!("Rename {title}"), &title, self.now);
                        self.open_layer(Layer::Search, SCOPE_SEARCH);
                        self.search.register(&mut self.focus);
                        return;
                    }
                    // never removed on one OK: the question comes first, and Keep it has the focus
                    "uninstall" => {
                        let title = app.get("title").and_then(Value::as_str).unwrap_or("").to_owned();
                        let icon = app.get("iconUrl").and_then(Value::as_str).unwrap_or("").to_owned();
                        self.confirm = Some(Confirm { app_id, title, icon, index: 0 });
                        self.open_layer(Layer::Confirm, SCOPE_CONFIRM);
                        self.menu_app = None;
                        return;
                    }
                    _ => {}
                }
                self.menu_app = None;
            }
            _ => {}
        }
    }

    // ---- the pointer
    fn pointer(&mut self, x: f32, y: f32, buttons: u32, visible: bool, out: &mut Vec<Effect>) {
        if !self.active {
            return;
        }
        if !visible {
            if self.focus.pointer_gone().from.is_some() {
                self.redraw(out);
            }
            self.pointer_down = false;
            self.pointer_armed = false;
            self.click_press = None;
            self.item_press = None;
            for (_, s) in &mut self.smarts {
                s.cancel_press();
                s.hover_item(None);
            }
            return;
        }
        self.cursor_visible = true;
        self.pointer_at = (x, y);
        let was = self.pointer_down;
        let down = buttons & 1 != 0;
        if !down {
            self.pointer_armed = true;
        }
        // a button down that was never seen going down is not a press
        let pressed = down && (was || self.pointer_armed);
        self.pointer_down = pressed;
        if pressed && !was {
            // a click: focuses what is under it and acts on it, as OK would
            if let Some(ed) = &mut self.editor {
                let outs = ed.press_at(x, y, &mut self.focus, self.now);
                self.edit_outs(outs, out);
                self.redraw(out);
                return;
            }
            // the popover: a click is the row it is on, and a click anywhere else puts it away
            if self.layer == Layer::Popover {
                match self.focus.click(x, y) {
                    Some(id) => self.popover_click(id, out),
                    None => self.close_layer(),
                }
                self.redraw(out);
                return;
            }
            // a history or a colour sheet: a click is on a chip, the ring, a track or a swatch, and a click beside the sheet puts it away
            if self.layer == Layer::OnHome && (self.history.is_some() || self.colour.is_some() || self.thermostat.is_some()) {
                self.sheet_pressed(x, y, out);
                self.redraw(out);
                return;
            }
            // an item of a smart-home card: the card is entered with the item in front, and the press lands when the button is let go over it
            if self.layer == Layer::None
                && let Some((wid, i)) = self.smart_hit(x, y)
            {
                self.press_item(&wid, i);
                self.redraw(out);
                return;
            }
            if let Some(id) = self.focus.hit(x, y).filter(|id| self.focus.is_live(*id)) {
                self.focus.click(x, y);
                self.click_press = Some((id, self.now));
            }
            return;
        }
        if !self.pointer_down && was {
            self.colour_held = None;
            let landed = self.click_press.take().map(|(id, _)| id);
            if let Some((wid, i, _, held)) = self.item_press.take() {
                let over = self.smart_of(&wid).and_then(|s| s.hit(x, y)) == Some(i);
                let (now, free) = (self.now, self.layer == Layer::None);
                let done = self.smart_mut(&wid).map(|s| {
                    if held || !over || !free {
                        s.cancel_press();
                        SmartOut::Nothing
                    } else {
                        s.press_up(now)
                    }
                });
                if let Some(done) = done {
                    self.smart_out(&wid, done, out);
                }
                self.redraw(out);
                return;
            }
            if let Some(ed) = &mut self.editor
                && let Some(o) = ed.release(x, y, &mut self.focus)
            {
                self.edit_outs(vec![o], out);
                self.redraw(out);
                return;
            }
            // the click: what the button went down on, released over the same control
            if let Some(id) = landed.filter(|id| self.focus.hit(x, y) == Some(*id)) {
                if self.layer == Layer::Apps {
                    let action = self.apps.clicked(id, &mut self.focus);
                    self.apps_out(action, out);
                    self.redraw(out);
                    return;
                }
                if self.layer == Layer::Search {
                    let result = self.search.clicked(id, &mut self.focus, self.now);
                    self.search_out(result, out);
                    self.redraw(out);
                    return;
                }
                if let Some(i) = id.0.checked_sub(CONFIRM_ID).filter(|i| (*i as usize) < CONFIRM_ANSWERS.len()).filter(|_| self.layer == Layer::Confirm) {
                    if let Some(c) = &mut self.confirm {
                        c.index = i as usize;
                    }
                    self.action("ok", out);
                    return;
                }
                if let Some(i) = Self::dock_index_of(id) {
                    self.dock.focus(i);
                    self.register_dock();
                }
                self.moved_at = self.now;
                self.action("ok", out);
            }
            return;
        }
        if !self.pointer_down && was {
            if let Some(ed) = &mut self.editor
                && let Some(o) = ed.release(x, y, &mut self.focus)
            {
                self.edit_outs(vec![o], out);
                self.redraw(out);
            }
            return;
        }
        if let Some(ed) = &mut self.editor
            && ed.drag(x, y)
        {
            self.redraw(out);
        }
        if self.pointer_down && self.layer == Layer::OnHome && self.colour_held.is_some() {
            self.colour_dragged(x, y, out);
            return;
        }
        let h = self.focus.hover(x, y, true);
        let mut moved = h.from != h.to;
        if self.layer == Layer::None {
            for (_, s) in &mut self.smarts {
                let at = s.hit(x, y);
                moved |= s.hover_item(at);
            }
        }
        if moved {
            self.redraw(out);
        }
    }

    fn edit_outs(&mut self, outs: Vec<EditorOut>, out: &mut Vec<Effect>) {
        for o in outs {
            match o {
                EditorOut::Preview(l) => self.preview_layout(l),
                EditorOut::Items(wid) => {
                    if let Some(s) = self.smart_mut(&wid) {
                        s.enter();
                    }
                }
                EditorOut::Save(l) => {
                    let file = grid::for_file(&l, &self.shipped_layout);
                    let text = serde_json::to_string_pretty(&file).unwrap_or_default() + "\n";
                    self.layout_text = file.to_string();
                    out.push(Effect::Write { path: "theme/layout.json".into(), text });
                    out.push(Effect::Log(format!("layout.json written, {} cards", grid::widgets(&file).len())));
                    self.apply_layout(file);
                    self.edit_end(out);
                }
                EditorOut::Leave(_) => {
                    self.apply_layout(serde_json::from_str(&self.layout_text).unwrap_or_else(|_| json!({})));
                    self.edit_end(out);
                }
            }
        }
    }

    /// Whether the weather card's picture may move: Home in front with the screen on, no screensaver over it, no layer over the cards, Home not
    /// calm ([`Home::set_calm`]) and the theme's `weather.animated` not off.
    pub fn weather_art_runs(&self) -> bool {
        self.active && self.screen_on && !self.saver_up && self.layer == Layer::None && !self.calm && self.weather_animated()
    }

    /// The theme's `weather.animated` (default on): the weather card's sprites move. The frame gate switches it off with the wallpaper's motion for the
    /// surfaces it measures at rest (`lumatv theme set '{"weather":{"animated":false}}'`): a card that draws every refresh keeps every flip of the screen
    /// "after an animating frame" and every burst of frames from ending, whatever the surface under test does.
    fn weather_animated(&self) -> bool {
        self.theme.pointer("/weather/animated").and_then(Value::as_bool).unwrap_or(true)
    }

    /// Whether the full weather view's picture may move: that view is up, Home in front, the screen on and no screensaver.
    pub fn weather_view_runs(&self) -> bool {
        self.active && self.screen_on && !self.saver_up && self.layer == Layer::Weather
    }

    /// Home is in front with nothing open on it: no layer, no layout editor, no card's inner focus, no tile being moved.
    pub fn at_rest(&self) -> bool {
        self.active && self.screen_on && self.layer == Layer::None && self.editor.is_none() && self.inner.is_none() && !self.dock.moving
    }

    /// What Home asked other kinds for since the last take.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// Whether the search sheet is over the library (opened from its Search button).
    pub fn search_over_apps(&self) -> bool {
        self.layer == Layer::Search && self.stack.contains(&Layer::Apps)
    }

    // ---- the view
    pub fn focus_view(&self) -> FocusView {
        FocusView { id: self.focus.current(), rested: self.now >= self.moved_at + self.rest_ms, hover: self.focus.hovered() }
    }

    /// Whether the view changed since it was last taken.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    /// Where the status bar's glass, the box of its buttons and the buttons are, for the tests and the harness's crops: `{pill: [x, y, w, h], group: [x, y, w, h], items: [[type, x, y, w, h], ...]}`.
    fn status_state(&self) -> Value {
        let (pill, group) = (self.status_pill(), self.status_group());
        let items: Vec<Value> = self.status_item_list().zip(self.status_frames()).map(|(it, f)| json!([kind_of(it), f.0, f.1, f.2, f.3])).collect();
        json!({"pill": [pill.x, pill.y, pill.w, pill.h], "group": [group.x, group.y, group.w, group.h], "items": items})
    }

    /// What `ctl shell` and the tests read.
    pub fn state(&self) -> Value {
        json!({
            "home": self.active,
            "layer": match self.layer { Layer::None => "", Layer::Apps => "apps", Layer::Search => "search", Layer::Edit => "edit", Layer::Picker => "picker", Layer::Menu => "menu", Layer::Confirm => "confirm", Layer::Popover => "popover", Layer::Weather => "weather", Layer::OnHome => "onhome" },
            "focus": self.focus.current().map(|i| i.0),
            "focusLayer": self.focus.current().map(|id| if Self::dock_index_of(id).is_some() { "dock" } else if self.widget_of(id).is_some() { if self.inner.is_some() { "widget" } else { "widgets" } } else { "statusbar" }),
            "widgetFocusId": self.focus.current().and_then(|id| self.widget_of(id)),
            "dockIndex": self.dock.index,
            "dockMoving": self.dock.moving,
            "editing": self.editor.is_some(),
            "editor": self.editor.as_ref().map(Editor::state),
            "search": if self.layer == Layer::Search { self.search.state() } else { Value::Null },
            "popover": self.popover.as_ref().map(|p| json!({"index": p.index, "first": p.first, "row": p.def().key, "values": popover::ROWS.iter().map(|d| popover::text(d, self.theme.get("wallpaperMotion").unwrap_or(&Value::Null))).collect::<Vec<_>>()})),
            "wallHole": self.wall_hole,
            "wall": self.wall.state(),
            "cards": self.cards().iter().map(|e| { let r = self.rect(e); json!([e.id, r.x, r.y, r.w, r.h]) }).collect::<Vec<_>>(),
            "layout": grid::entries(&self.layout).iter().map(|e| json!([e.id, e.col, e.row, e.span, e.rows, e.hidden as u8])).collect::<Vec<_>>(),
            "dock": self.dock.ids(),
            "profile": self.profile_name(),
            "unread": self.unread,
            "cameraFrames": self.stills.revision,
            "statusBar": self.status_state(),
            "onhome": json!({
                "tiles": self.onhome_tile_count(),
                "items": self.smarts.first().map(|(_, s)| s.items().len()),
                "focus": self.smarts.first().and_then(|(_, s)| s.focus_id()),
                "offset": self.smarts.first().map(|(_, s)| s.offset()),
                "sheet": self.sheet_id(),
                "sheetKind": if self.sheet.is_some() { "controls" } else if self.history.is_some() { "history" } else if self.colour.is_some() { "colour" } else if self.thermostat.is_some() { "thermostat" } else { "" },
                "notice": self.onhome_notice(),
            }),
            "apps": self.apps.state(),
            "picker": self.picker.state(),
            "launching": self.launching,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> Home {
        let theme = lumatv_tokens::Tokens::shipped().json().clone();
        let shipped: Value = serde_json::from_str(include_str!("../../../../payload/theme/layout.json")).unwrap();
        let mut h = Home::new(theme, shipped, json!({}));
        h.start(1000);
        h.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 1000);
        let mut lps = vec![json!({"id": APPS_TILE, "title": "Apps", "unmovable": true, "icon": "/a.png"})];
        for (id, t) in [("netflix", "Netflix"), ("youtube", "YouTube"), ("plex", "Plex")] {
            lps.push(json!({"id": id, "title": t, "icon": format!("/{id}.png"), "iconColor": "#101010", "removable": true}));
        }
        h.update(&Input::Luna { tag: LP_TAG, payload: json!({"launchPoints": lps}) }, 1001);
        h
    }
    #[test]
    fn theme_poll_keeps_preview_and_adopts_unrelated_disk_changes() {
        let mut h = home();
        h.set_theme_preview(Some(json!({"clock": {"style": "preview"}})));
        h.update(&Input::Fetched { tag: THEME_TAG, status: 200, data: json!({"clock": {"style": "digital", "other": 1}}) }, 1100);
        assert_eq!(h.theme["clock"]["style"], "preview");
        assert_eq!(h.theme["clock"]["other"], 1);
        h.update(&Input::Fetched { tag: THEME_TAG, status: 200, data: json!({"clock": {"style": "analog", "other": 2}}) }, 1200);
        assert_eq!(h.theme["clock"]["style"], "preview");
        assert_eq!(h.theme["clock"]["other"], 2);
    }

    #[test]
    fn the_chrome_revision_moves_with_the_layout_and_the_themes_clock_and_tiles_only() {
        let mut h = home();
        let (layout, theme) = h.chrome_rev();
        let mut file = h.theme.clone();
        file["wallpaperMotion"] = json!({"speed": 2});
        h.set_theme(file.clone());
        assert_eq!(h.chrome_rev(), (layout, theme), "a key the chrome is not made from");
        file["clock"] = json!({"style": "restyled"});
        h.set_theme(file.clone());
        assert_eq!(h.chrome_rev(), (layout, theme + 1));
        file["tiles"] = json!({"inset": 3});
        h.set_theme(file);
        assert_eq!(h.chrome_rev(), (layout, theme + 2));
        h.preview_layout(h.layout.clone());
        assert_eq!(h.chrome_rev().0, layout, "the layout it already has");
        let mut next = h.layout.clone();
        next["grid"]["rows"] = json!(20);
        h.preview_layout(next);
        assert_eq!(h.chrome_rev().0, layout + 1);
        h.apply_layout(json!({"grid": {"rows": 18}}));
        assert_eq!(h.chrome_rev().0, layout + 2);
    }


    fn press(h: &mut Home, name: &str, at: u64) -> Vec<Effect> {
        let mut out = h.update(&Input::Key { name: name.into(), down: true, repeat: false }, at);
        out.extend(h.update(&Input::Key { name: name.into(), down: false, repeat: false }, at + 30));
        out
    }

    impl Home {
        /// The arrows from where Home opens (the status bar's first button, over the weather card) to the card `widget`: down onto the weather
        /// card, then left along the cards, and down where the left runs out.
        pub(crate) fn walk_to_card(&mut self, widget: &str, at: u64) {
            let mut t = at;
            let mut tap = |h: &mut Home, key: &str| {
                h.update(&Input::Key { name: key.into(), down: true, repeat: false }, t);
                h.update(&Input::Key { name: key.into(), down: false, repeat: false }, t + 30);
                t += 100;
            };
            tap(self, "down");
            for _ in 0..8 {
                if self.focus.current().and_then(|id| self.widget_of(id)) == Some(widget) {
                    return;
                }
                let before = self.focus.current();
                tap(self, "left");
                if self.focus.current() == before {
                    tap(self, "down");
                }
            }
        }
    }

    /// The arrows from where Home opens to the dock's tile under the home card: down onto the weather card, left past the news card to the
    /// home card, down.
    fn to_the_dock_under_the_home_card(h: &mut Home, at: u64) {
        for (i, key) in ["down", "left", "left", "down"].into_iter().enumerate() {
            press(h, key, at + i as u64 * 100);
        }
    }

    /// The helper's `plugins` topic as the TV answers it, with the roster of the helper's own plugins and the owner's.
    fn roster(owner: Value) -> Value {
        let own = |id: &str| json!({"class": "S", "firstParty": true, "id": id, "name": id, "registered": true, "state": "connected", "version": "1"});
        json!({"firstParty": [own("weather"), own("news"), own("clock")], "plugins": owner, "problem": null})
    }

    #[test]
    fn ok_on_the_weather_card_opens_the_weather_view_left_and_right_walk_it_and_back_closes() {
        let mut h = home();
        // down from the status bar to the cards, then right along them to the weather card
        press(&mut h, "down", 1100);
        for i in 0..6 {
            if h.focus.current().and_then(|id| h.widget_of(id)) == Some("weather") {
                break;
            }
            press(&mut h, "right", 1200 + i * 100);
        }
        assert_eq!(h.focus.current().and_then(|id| h.widget_of(id)), Some("weather"), "the focus reached the weather card");
        press(&mut h, "ok", 2000);
        assert_eq!(h.layer, Layer::Weather);
        assert_eq!(h.state()["layer"], "weather");
        assert!(h.weather_view_runs() && !h.weather_art_runs(), "the view's picture moves, the card's under it does not");
        assert_eq!(h.wview.page, super::super::weather_view::Page::Now);
        press(&mut h, "left", 2100);
        assert_eq!(h.wview.page, super::super::weather_view::Page::Now, "nothing before the first page");
        press(&mut h, "down", 2150);
        assert_eq!(h.wview.page, super::super::weather_view::Page::Now, "Down no longer pages");
        press(&mut h, "right", 2200);
        assert_eq!(h.wview.page, super::super::weather_view::Page::Radar);
        assert!(h.radar.open || h.weather.pointer("/doc/lat").is_none(), "the helper's radar is asked for when the page comes (with a place to ask for)");
        press(&mut h, "right", 2250);
        assert_eq!(h.wview.page, super::super::weather_view::Page::Details);
        assert!(!h.radar.open, "and let go when the page goes");
        press(&mut h, "right", 2400);
        assert_eq!(h.wview.page, super::super::weather_view::Page::Details, "nor is there a page after the last");
        press(&mut h, "down", 2450);
        press(&mut h, "left", 2460);
        assert_eq!(h.wview.page, super::super::weather_view::Page::Radar, "Left goes back");
        press(&mut h, "right", 2470);
        assert_eq!(h.wview.page, super::super::weather_view::Page::Details);
        press(&mut h, "back", 2500);
        assert_eq!(h.layer, Layer::None, "Back closes it");
        assert!(h.weather_art_runs() && !h.weather_view_runs());
        // opened again it starts on the first page; a reading that arrives while it is up draws it again
        press(&mut h, "ok", 3000);
        assert_eq!(h.wview.page, super::super::weather_view::Page::Now);
        let out = h.update(&Input::Helper { topic: "weather.more".into(), data: json!({"doc": {"hourly": []}}) }, 3100);
        assert!(out.contains(&Effect::Redraw));
    }

    #[test]
    fn on_the_map_the_wheel_and_up_and_down_zoom_between_the_whole_earth_and_the_closest_view_and_left_and_right_still_page() {
        use super::super::radar_zoom::{KEY_STEP, WHEEL_STEP, Z_GLOBE, Z_HOME, Z_MAX};
        use super::super::weather_view::Page;
        let mut h = home();
        h.update(&Input::Helper { topic: "weather".into(), data: json!({"doc": {"lat": 33.75, "lon": -84.39, "display": {"place": "Atlanta"}}}) }, 1002);
        press(&mut h, "down", 1100);
        for i in 0..6 {
            if h.focus.current().and_then(|id| h.widget_of(id)) == Some("weather") {
                break;
            }
            press(&mut h, "right", 1200 + i * 100);
        }
        press(&mut h, "ok", 2000);
        let wheel = |h: &mut Home, name: &str, at: u64| {
            h.update(&Input::Key { name: name.into(), down: true, repeat: false }, at);
            h.update(&Input::Key { name: name.into(), down: false, repeat: false }, at + 5);
        };
        wheel(&mut h, "wheelup", 2050);
        assert!(!h.radar.open && h.radar.zoom.target == Z_HOME, "the wheel does nothing off the map");
        assert!(!h.radar_takes_keys(), "and the compositor is not asked for it there");
        press(&mut h, "right", 2100);
        assert_eq!(h.wview.page, Page::Radar);
        assert!(h.radar.open && h.radar_takes_keys(), "on the map the wheel and the channel keys are Home's");
        assert!(h.radar.zoom_by(0.0, None, 0).is_empty() && h.radar.zoom.target == Z_HOME, "it opens at the home view");
        // wheel up zooms in a notch, wheel down out, about the middle of the map with no pointer
        wheel(&mut h, "wheelup", 2200);
        assert_eq!((h.radar.zoom.target, h.radar.zoom.anchor), (Z_HOME + WHEEL_STEP, None));
        wheel(&mut h, "wheeldown", 2300);
        wheel(&mut h, "wheeldown", 2400);
        assert_eq!(h.radar.zoom.target, Z_HOME - WHEEL_STEP);
        // with the pointer shown the zoom is about it
        h.update(&Input::Pointer { x: 1500.0, y: 300.0, buttons: 0, visible: true }, 2450);
        wheel(&mut h, "wheelup", 2500);
        assert_eq!((h.radar.zoom.target, h.radar.zoom.anchor), (Z_HOME, Some((1500.0, 300.0))));
        // Up and Down zoom the same way, a step each
        press(&mut h, "up", 2600);
        assert_eq!((h.radar.zoom.target, h.radar.zoom.anchor), (Z_HOME + KEY_STEP, None));
        press(&mut h, "down", 2700);
        assert_eq!(h.radar.zoom.target, Z_HOME);
        assert_eq!(h.wview.page, Page::Radar, "neither pages");
        // the ends: no further than the closest view, and no further out than the whole earth
        for i in 0..20 {
            wheel(&mut h, "wheelup", 3000 + i * 10);
        }
        assert_eq!(h.radar.zoom.target, Z_MAX);
        wheel(&mut h, "wheelup", 3300);
        assert_eq!(h.radar.zoom.target, Z_MAX, "the clamp holds");
        for i in 0..40 {
            wheel(&mut h, "wheeldown", 4000 + i * 10);
        }
        assert_eq!(h.radar.zoom.target, Z_GLOBE);
        assert!(h.radar.globe(), "all the way out is the globe");
        wheel(&mut h, "wheeldown", 5000);
        assert_eq!((h.radar.zoom.target, h.wview.page), (Z_GLOBE, Page::Radar), "and a wheel past it neither zooms nor leaves the page");
        // the digits are no keys of the view now, and Left and Right page
        press(&mut h, "3", 5100);
        assert_eq!(h.radar.zoom.target, Z_GLOBE);
        press(&mut h, "left", 5200);
        assert_eq!(h.wview.page, Page::Now);
        assert!(!h.radar.open && h.radar.zoom.target == Z_HOME, "leaving the map lets its view go: it opens at the home view again");
    }

    #[test]
    fn a_calm_home_rests_the_weather_and_stills_the_wallpaper() {
        let mut h = home();
        assert!(h.weather_art_runs() && !h.wallpaper_held(), "Home alone: everything moves");
        assert!(h.set_calm(true), "something is over it: it changed");
        assert!(!h.weather_art_runs(), "the weather's sprites rest");
        assert!(h.wallpaper_held(), "the wallpaper is held");
        assert!(!h.set_calm(true), "the same again changes nothing, so no frame is asked for");
        assert!(h.set_calm(false));
        assert!(h.weather_art_runs() && !h.wallpaper_held(), "back to Home alone");
    }

    #[test]
    fn the_themes_weather_animated_switch_rests_the_sprites_and_nothing_else() {
        let mut h = home();
        assert!(h.weather_art_runs(), "on unless the theme says otherwise");
        let mut theme = h.theme.clone();
        theme["weather"]["animated"] = json!(false);
        h.set_theme(theme);
        assert!(!h.weather_art_runs(), "off: the sprites rest");
        assert!(!h.wallpaper_held(), "the wallpaper's own switch is its own");
        let mut theme = h.theme.clone();
        theme["weather"]["animated"] = json!(true);
        h.set_theme(theme);
        assert!(h.weather_art_runs());
    }

    /// A Home whose layout has a smart-home card on it, fed the helper's `onhome` topic.
    fn smart_home(tiles: Value) -> Home {
        let theme = lumatv_tokens::Tokens::shipped().json().clone();
        let shipped: Value = serde_json::from_str(include_str!("../../../../payload/theme/layout.json")).unwrap();
        let layout = json!({"grid": {"rows": 16}, "widgets": [{"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 6, "rows": 4}, {"id": "smart", "type": "onhome", "col": 0, "row": 4, "span": 6, "rows": 5, "title": "Smart home"},
                                         {"id": "weather", "type": "weather", "col": 6, "row": 0, "span": 6, "rows": 9},
                                         {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]});
        let mut h = Home::new(theme, shipped, layout);
        h.start(1000);
        h.update(&Input::Topic { topic: "foreground".into(), data: json!({"app": "com.webos.app.home", "home": true, "covered": false}) }, 1000);
        h.update(&Input::Luna { tag: LP_TAG, payload: json!({"launchPoints": [json!({"id": APPS_TILE, "title": "Apps", "unmovable": true, "icon": "/a.png"})]}) }, 1001);
        h.update(&Input::Helper { topic: "onhome".into(), data: json!({"state": "connected", "words": "3 things", "tiles": tiles}) }, 1002);
        h
    }

    fn lamp() -> Value {
        json!({"id": "light.lamp", "name": "Lamp", "group": "lights", "words": "On \u{b7} 50%", "tone": "on", "icon": "lightbulb", "available": true, "can": ["toggle"], "verbs": ["turn_on", "turn_off", "toggle"],
               "ctl": {"on": true, "dimmable": true, "brightness": 50, "kelvin": null}})
    }

    fn thermostat() -> Value {
        json!({"id": "climate.t", "name": "Thermostat", "group": "climate", "words": "68\u{b0}", "tone": "off", "icon": "thermometer", "available": true, "can": ["sheet"], "verbs": ["set_temperature", "set_hvac_mode"],
               "ctl": {"mode": "heat", "modes": ["off", "heat"], "current": 68.0, "target": 70.0, "min": 50.0, "max": 90.0, "step": 1.0, "unit": "\u{b0}F", "action": ""}})
    }

    fn sensor() -> Value {
        json!({"id": "sensor.t", "name": "Hall", "group": "sensors", "words": "21\u{b0}C", "tone": "info", "icon": "thermometer", "available": true, "can": [], "verbs": [], "ctl": {}})
    }

    fn into_smart_card(h: &mut Home) {
        h.walk_to_card("smart", 1100);
        assert_eq!(h.focus.current().and_then(|id| h.widget_of(id)), Some("smart"), "the focus reached the smart-home card");
        press(h, "ok", 2000);
        assert_eq!(h.inner.as_deref(), Some("smart"), "OK on the card enters it");
    }

    /// The body of the act a press sent, whatever the tag it was sent under.
    fn act_of(out: &[Effect]) -> Option<Value> {
        out.iter().find_map(|e| match e {
            Effect::Request { method, path, body, .. } if method == "POST" && path == "/v1/onhome/act" => body.clone(),
            _ => None,
        })
    }

    fn act_tag_of(out: &[Effect]) -> Option<u64> {
        out.iter().find_map(|e| match e {
            Effect::Request { tag, method, path, .. } if method == "POST" && path == "/v1/onhome/act" => Some(*tag),
            _ => None,
        })
    }

    /// Whether the card shows the item on, as it shows it now (the topic's, or what a press put in its place).
    fn item_on(h: &Home, id: &str) -> Option<bool> {
        h.smart_of("smart")?.items().iter().find(|i| i.id == id).map(|i| i.on)
    }

    fn item_in_front(h: &Home) -> Option<usize> {
        h.smart_of("smart").and_then(Smart::focused)
    }

    /// Puts the item `id` in front with the arrows, whatever the layout is. Returns the time after the last key.
    fn go_to_item(h: &mut Home, id: &str, mut at: u64) -> u64 {
        for _ in 0..30 {
            let s = h.smart_of("smart").expect("the card's state");
            let want = s.items().iter().position(|i| i.id == id).expect("the item");
            let cur = s.focused().expect("the card is entered");
            if cur == want {
                return at;
            }
            let ((row, col), (to_row, to_col)) = ((s.plan().placed[cur].row, s.plan().placed[cur].col), (s.plan().placed[want].row, s.plan().placed[want].col));
            let key = if to_row > row { "down" } else if to_row < row { "up" } else if to_col > col { "right" } else { "left" };
            press(h, key, at);
            at += 10;
        }
        panic!("the arrows did not reach {id}");
    }

    #[test]
    fn the_smart_home_card_is_entered_with_ok_the_arrows_move_among_its_items_and_back_leaves_it() {
        let mut h = smart_home(json!([lamp(), thermostat(), sensor()]));
        assert_eq!(item_in_front(&h), None, "no item is in front until the card is entered");
        into_smart_card(&mut h);
        assert_eq!(item_in_front(&h), Some(0), "the first item is in front");
        let mut at = 2100;
        for id in ["climate.t", "sensor.t", "light.lamp"] {
            at = go_to_item(&mut h, id, at);
        }
        // an arrow that has nowhere to go leaves the item in front where it is, and the card entered
        for k in ["up", "left", "left"] {
            press(&mut h, k, 2500);
        }
        assert_eq!((item_in_front(&h), h.inner.as_deref()), (Some(0), Some("smart")));
        // Back leaves the card; entered again, the item that was in front is in front
        press(&mut h, "back", 3000);
        assert!(h.inner.is_none());
        press(&mut h, "ok", 3100);
        assert_eq!((h.inner.as_deref(), item_in_front(&h)), (Some("smart"), Some(0)));
        assert!(h.take_requests().is_empty(), "entering is not a press on an item");
    }

    #[test]
    fn the_menu_key_on_the_entered_card_opens_the_menu_of_the_item_in_front_and_not_the_apps() {
        let mut h = smart_home(json!([lamp(), thermostat()]));
        into_smart_card(&mut h);
        press(&mut h, "menu", 2100);
        assert_eq!(h.layer, Layer::None, "the apps are not opened over the card");
        assert!(h.smart_of("smart").is_some_and(|s| s.arrange.is_some()), "the item's menu is open");
        // Back closes the menu and leaves the card entered; the next Back leaves the card
        press(&mut h, "back", 2200);
        assert!(h.smart_of("smart").is_some_and(|s| s.arrange.is_none()));
        assert_eq!(h.inner.as_deref(), Some("smart"));
        press(&mut h, "back", 2300);
        assert!(h.inner.is_none());
        // off the card, Menu is the apps, as it was
        press(&mut h, "menu", 2400);
        assert_eq!(h.layer, Layer::Apps);
    }

    #[test]
    fn ok_on_a_lamp_turns_it_at_once_a_refusal_puts_it_back_and_says_why_and_an_answer_keeps_it() {
        let mut h = smart_home(json!([lamp(), thermostat(), sensor()]));
        into_smart_card(&mut h);
        assert_eq!(item_on(&h, "light.lamp"), Some(true));
        let out = press(&mut h, "ok", 2100);
        assert_eq!(act_of(&out), Some(json!({"id": "light.lamp", "verb": "toggle", "value": null})));
        assert_eq!(item_on(&h, "light.lamp"), Some(false), "shown off in the step of the key, before Home Assistant has said anything");
        assert_eq!(h.layer, Layer::None);
        // refused: the lamp is what the topic says again, and the card says why for a moment
        let tag = act_tag_of(&out).expect("a request was made");
        let refused = json!({"ok": false, "error": {"code": "unavailable", "message": "Home Assistant cannot reach it right now"}});
        h.update(&Input::Fetched { tag, status: 409, data: refused }, 2200);
        assert_eq!(item_on(&h, "light.lamp"), Some(true));
        assert_eq!(h.onhome_notice(), Some("Home Assistant cannot reach it right now"));
        h.update(&Input::Tick, 6300);
        assert_eq!(h.onhome_notice(), None, "the words go by themselves");
        // done: it stays as shown while the topic catches up, and follows the topic after
        let out = press(&mut h, "ok", 6400);
        let tag = act_tag_of(&out).expect("a request was made");
        h.update(&Input::Fetched { tag, status: 200, data: json!({"ok": true}) }, 6500);
        assert_eq!((item_on(&h, "light.lamp"), h.onhome_notice()), (Some(false), None));
        let mut off = lamp();
        off["ctl"]["on"] = json!(false);
        h.update(&Input::Helper { topic: "onhome".into(), data: json!({"state": "connected", "tiles": [off, thermostat(), sensor()]}) }, 6600);
        assert_eq!(item_on(&h, "light.lamp"), Some(false));
    }

    #[test]
    fn ok_on_a_camera_opens_the_cameras_viewer_and_a_held_ok_on_a_lamp_is_not_a_press() {
        let mut h = smart_home(json!([lamp()]));
        h.update(&Input::Helper { topic: "cameras".into(), data: json!({"cameras": [{"id": "front", "name": "Front door", "online": true}]}) }, 1100);
        into_smart_card(&mut h);
        assert_eq!(h.smart_of("smart").map(|s| s.items()[0].id.clone()), Some("camera:front".to_owned()), "a camera comes first");
        let out = press(&mut h, "ok", 2100);
        assert!(act_of(&out).is_none(), "a camera is not operated");
        assert_eq!(h.take_requests(), vec![Request::CameraFull("front".to_owned())]);
        // the lamp: held, OK opens what is under a hold and sends nothing; its release is not a press
        go_to_item(&mut h, "light.lamp", 2200);
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 3000);
        let out = h.update(&Input::Tick, 3800);
        assert!(act_of(&out).is_none(), "a hold is not an OK");
        let layer = h.layer;
        h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 3850);
        assert_eq!(h.layer, layer, "the release of a hold presses nothing");
        assert_eq!(item_on(&h, "light.lamp"), Some(true), "and the lamp is as it was");
    }

    fn number_sensor() -> Value {
        json!({"id": "sensor.t", "name": "Hall", "group": "sensors", "words": "21.4\u{b0}C", "tone": "info", "icon": "thermometer", "available": true, "can": [], "verbs": [],
               "ctl": {"kind": "number", "value": 21.4, "unit": "\u{b0}C", "class": "temperature"}})
    }

    fn history_doc(range: &str) -> Value {
        json!({"history": {"id": "sensor.t", "kind": "number", "range": range, "from": 1000.0, "to": 87_400_000.0, "unit": "\u{b0}C", "class": "temperature", "source": "history",
                           "points": [[1000.0, 20.0], [2000.0, 22.0]], "stats": {"min": 20.0, "max": 22.0, "avg": 21.0, "min_at": 1000.0, "max_at": 2000.0, "now": 22.0, "now_at": 2000.0}}})
    }

    fn history_request(out: &[Effect]) -> Option<String> {
        out.iter().find_map(|e| match e {
            Effect::Request { tag, method, path, .. } if *tag == HISTORY_TAG && method == "GET" => Some(path.clone()),
            _ => None,
        })
    }

    #[test]
    fn ok_on_a_sensor_opens_its_history_the_helpers_answer_fills_it_left_and_right_walk_the_ranges_and_back_closes_it() {
        use super::super::onhome::sensor::Load;
        let mut h = smart_home(json!([lamp(), number_sensor()]));
        into_smart_card(&mut h);
        go_to_item(&mut h, "sensor.t", 2100);
        let out = press(&mut h, "ok", 2500);
        assert_eq!(history_request(&out).as_deref(), Some("/v1/onhome/history?id=sensor.t&range=24h"), "the history is asked for as the sheet opens");
        assert_eq!((h.layer, h.sheet.is_none(), h.history.as_ref().map(|s| s.load.clone())), (Layer::OnHome, true, Some(Load::Asking)), "it is not the controls sheet");
        assert_eq!(h.state()["onhome"]["sheetKind"], "history");
        h.update(&Input::Fetched { tag: HISTORY_TAG, status: 200, data: history_doc("24h") }, 2600);
        assert_eq!(h.history.as_ref().map(|s| s.load.clone()), Some(Load::Ready));
        // Right is the next range, asked for; the answer for the one that was left is not taken
        let out = press(&mut h, "right", 2700);
        assert_eq!(history_request(&out).as_deref(), Some("/v1/onhome/history?id=sensor.t&range=7d"));
        h.update(&Input::Fetched { tag: HISTORY_TAG, status: 200, data: history_doc("24h") }, 2800);
        assert_eq!(h.history.as_ref().map(|s| s.load.clone()), Some(Load::Asking));
        // a refusal says why, in the helper's words
        h.update(&Input::Fetched { tag: HISTORY_TAG, status: 503, data: json!({"error": {"message": "Home Assistant is not linked"}}) }, 2900);
        assert_eq!(h.history.as_ref().map(|s| s.load.clone()), Some(Load::Failed("Home Assistant is not linked".into())));
        let out = press(&mut h, "ok", 2950);
        assert!(history_request(&out).is_none() && act_of(&out).is_none(), "OK on a sheet that only shows does nothing");
        press(&mut h, "back", 3000);
        assert_eq!((h.layer, h.history.is_none()), (Layer::None, true), "Back closes it");
        // opened again, and the sensor leaves Home: the sheet goes with it
        press(&mut h, "ok", 3100);
        assert!(h.history.is_some());
        h.update(&Input::Helper { topic: "onhome".into(), data: json!({"state": "connected", "tiles": [lamp()]}) }, 3200);
        assert_eq!((h.layer, h.history.is_none()), (Layer::None, true));
    }

    #[test]
    fn a_held_ok_on_a_colour_lamp_opens_its_colour_sheet_and_what_is_changed_is_sent_a_quarter_of_a_second_apart() {
        let mut h = smart_home(json!([colour_lamp()]));
        into_smart_card(&mut h);
        go_to_item(&mut h, "light.strip", 2100);
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 3000);
        h.update(&Input::Tick, 3800);
        h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 3850);
        assert_eq!((h.layer, h.sheet.is_none(), h.colour.as_ref().map(|c| c.id.clone())), (Layer::OnHome, true, Some("light.strip".to_owned())), "the colour sheet, not the controls");
        assert_eq!(h.state()["onhome"]["sheetKind"], "colour");
        // the row in front is the hue: Right turns it and the call goes at once, with the colour
        let out = press(&mut h, "right", 4000);
        let body = act_of(&out).expect("a call");
        assert_eq!((body["id"].as_str(), body["verb"].as_str()), (Some("light.strip"), Some("colour")));
        assert_eq!(body["value"]["rgb"].as_array().map(Vec::len), Some(3));
        // a held key: more changes within the quarter of a second wait, and the last one is sent when it is up
        assert!(act_of(&press(&mut h, "right", 4050)).is_none());
        assert!(act_of(&press(&mut h, "right", 4100)).is_none());
        assert!(h.due_at().is_some_and(|d| d <= 4250), "the call that waits is due");
        let out = h.update(&Input::Tick, 4260);
        assert!(act_of(&out).is_some(), "sent");
        // the next row, brightness, has its own verb
        press(&mut h, "down", 4300);
        press(&mut h, "down", 4310);
        let out = press(&mut h, "left", 4600);
        assert_eq!(act_of(&out).map(|b| b["verb"].clone()), Some(json!("brightness")));
        // Back closes it, and the lamp's own state is the card's again
        press(&mut h, "back", 5000);
        assert_eq!((h.layer, h.colour.is_none()), (Layer::None, true));
    }

    fn colour_lamp() -> Value {
        json!({"id": "light.strip", "name": "Strip", "group": "lights", "words": "On \u{b7} 60%", "tone": "on", "icon": "lightbulb", "available": true, "can": ["toggle"],
               "verbs": ["turn_on", "turn_off", "toggle", "colour", "brightness", "effect"],
               "ctl": {"on": true, "dimmable": true, "brightness": 60, "colour": true, "rgb": [255, 0, 128], "kelvin": null, "effect": "Rainbow", "effects": ["None", "Rainbow", "Pulse"]}})
    }

    #[test]
    fn the_pointer_picks_on_the_colour_sheet_and_a_click_beside_it_puts_it_away() {
        use super::super::onhome::{colour, colour_surface};
        let mut h = smart_home(json!([colour_lamp()]));
        into_smart_card(&mut h);
        go_to_item(&mut h, "light.strip", 2100);
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 3000);
        h.update(&Input::Tick, 3800);
        h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 3850);
        let (rows, effects) = h.colour.as_ref().map(|c| (c.rows.clone(), c.effects.len())).expect("the sheet");
        let lay = colour_surface::layout(&h.onhome_metrics, &rows, effects);
        let sat = lay.rows.iter().find(|r| r.row == colour::Row::Saturation).expect("a saturation track");
        let at = |t: f32| (lay.pane.x + sat.track.x + sat.track.w * t, lay.pane.y + sat.track.y + sat.track.h / 2.0);
        let click = |h: &mut Home, (x, y): (f32, f32), now: u64| {
            h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, now);
            let out = h.update(&Input::Pointer { x, y, buttons: 1, visible: true }, now + 10);
            let mut up = h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, now + 20);
            up.extend(out);
            up
        };
        let out = click(&mut h, at(0.25), 4000);
        assert!((h.colour.as_ref().unwrap().sat - 0.25).abs() < 0.01, "a press on the track sets the row to where it is");
        assert_eq!(act_of(&out).map(|b| b["verb"].clone()), Some(json!("colour")));
        // held, the thumb follows the pointer along the track, and past its end
        let (x, y) = at(0.5);
        h.update(&Input::Pointer { x, y, buttons: 1, visible: true }, 4400);
        h.update(&Input::Pointer { x: x + 900.0, y: y + 30.0, buttons: 1, visible: true }, 4450);
        assert_eq!(h.colour.as_ref().unwrap().sat, 1.0);
        h.update(&Input::Pointer { x: x + 900.0, y: y + 30.0, buttons: 0, visible: true }, 4500);
        // a click beside the sheet is Back
        click(&mut h, (5.0, 5.0), 5000);
        assert_eq!((h.layer, h.colour.is_none()), (Layer::None, true));
    }

    /// The owner's own tiles (the helper's own words for his real lamps, fans and thermostats: `onhome::owner_tile`) as a card's topic.
    fn owner(ids: &[&str]) -> Value {
        Value::Array(ids.iter().map(|id| super::super::onhome::owner_tile(id)).collect())
    }

    /// OK held as long as a hold is, and let go.
    fn hold_ok(h: &mut Home, at: u64) -> Vec<Effect> {
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, at);
        let out = h.update(&Input::Tick, at + 800);
        h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, at + 850);
        out
    }

    #[test]
    fn a_short_ok_on_a_lamp_a_fan_and_a_switch_switches_it_at_once_and_opens_no_menu_and_no_sheet() {
        let pump = json!({"id": "switch.pump", "name": "Pump", "group": "switches", "words": "Off", "tone": "off", "icon": "power", "available": true, "can": ["toggle"], "verbs": ["turn_on", "turn_off", "toggle"], "ctl": {"on": false}});
        let mut tiles = owner(&["light.master_bedroom_corner_light", "fan.living_room_vent_fan"]);
        tiles.as_array_mut().unwrap().push(pump);
        let mut h = smart_home(tiles);
        into_smart_card(&mut h);
        let mut at = 2100;
        for (id, was) in [("light.master_bedroom_corner_light", true), ("fan.living_room_vent_fan", true), ("switch.pump", false)] {
            at = go_to_item(&mut h, id, at) + 100;
            assert_eq!(item_on(&h, id), Some(was), "{id}");
            let out = press(&mut h, "ok", at);
            assert_eq!(act_of(&out), Some(json!({"id": id, "verb": "toggle", "value": null})), "{id}: one control, sent in the step of the key");
            assert_eq!(item_on(&h, id), Some(!was), "{id}: shown switched before Home Assistant has said anything");
            assert!(h.layer == Layer::None && h.sheet.is_none() && h.history.is_none() && h.colour.is_none() && h.thermostat.is_none(), "{id}: no sheet, no other screen");
            assert!(h.smart_of("smart").is_some_and(|s| s.arrange.is_none()), "{id}: no item menu");
            at += 100;
        }
    }

    #[test]
    fn a_held_ok_on_a_colour_bulb_opens_its_colour_sheet_with_the_wheel_the_white_range_and_the_effects_and_switches_nothing() {
        use super::super::onhome::colour::{Mode as Lit, Row};
        let mut h = smart_home(owner(&["light.master_bedroom_corner_light", "light.living_room_floor_lamp", "light.kitchen_lights"]));
        into_smart_card(&mut h);
        let at = go_to_item(&mut h, "light.master_bedroom_corner_light", 2100) + 100;
        let out = hold_ok(&mut h, at);
        assert!(act_of(&out).is_none() && item_on(&h, "light.master_bedroom_corner_light") == Some(true), "a hold is not an OK");
        assert_eq!((h.layer, h.sheet.is_none(), h.state()["onhome"]["sheetKind"].clone()), (Layer::OnHome, true, json!("colour")), "the colour sheet, not the controls");
        let c = h.colour.as_ref().expect("the colours");
        assert_eq!(c.rows, [Row::Hue, Row::Saturation, Row::Brightness, Row::Temperature, Row::Preset, Row::Effect]);
        assert_eq!((c.kelvin_range, c.effects.len(), c.mode), ((2000.0, 6535.0), 20, Lit::White), "the bulb's own white range and effects, and a white that is showing");
        assert!((c.hue - 30.601).abs() < 0.01);
        // Back, and the floor lamp that is off: held, it opens its sheet too, with the hundred and eleven effects
        press(&mut h, "back", at + 1000);
        let at = go_to_item(&mut h, "light.living_room_floor_lamp", at + 1100) + 100;
        hold_ok(&mut h, at);
        assert_eq!(h.colour.as_ref().map(|c| (c.effects.len(), c.kelvin_range)), Some((111, (2700.0, 6500.0))));
        press(&mut h, "back", at + 1000);
        // a lamp that only dims: the sheet has the one row
        let at = go_to_item(&mut h, "light.kitchen_lights", at + 1100) + 100;
        hold_ok(&mut h, at);
        assert_eq!(h.colour.as_ref().map(|c| c.rows.clone()), Some(vec![Row::Brightness]));
    }

    #[test]
    fn a_short_ok_on_a_lamp_the_tv_may_not_switch_says_why_and_opens_nothing() {
        // as the helper makes it while Home Assistant allows the TV no control of lights (what the owner had)
        let mut tile = super::super::onhome::owner_tile("light.master_bedroom_corner_light");
        tile["can"] = json!([]);
        tile["verbs"] = json!([]);
        tile["why"] = json!("Home Assistant does not let the TV control this");
        let mut h = smart_home(json!([tile]));
        into_smart_card(&mut h);
        let out = press(&mut h, "ok", 2100);
        assert!(act_of(&out).is_none(), "nothing to send");
        assert_eq!(h.onhome_notice(), Some("Home Assistant does not let the TV control this"));
        assert!(h.layer == Layer::None && h.sheet.is_none() && h.colour.is_none(), "and no screen with nothing on it");
        assert!(h.smart_of("smart").is_some_and(|s| s.arrange.is_none()));
        hold_ok(&mut h, 2300);
        assert!(h.layer == Layer::None && h.colour.is_none(), "a hold has nothing under it either");
        h.update(&Input::Tick, 8000);
        assert_eq!(h.onhome_notice(), None, "the words go by themselves");
    }

    #[test]
    fn ok_on_a_thermostat_opens_its_own_sheet_and_a_step_asks_for_set_temperature_when_it_has_rested_and_a_mode_when_chosen() {
        use super::super::onhome::thermostat::Row as ThermostatRow;
        let mut bedjet = super::super::onhome::owner_tile("climate.master_bedroom_bedjet");
        bedjet["ctl"]["mode"] = json!("heat");
        let mut h = smart_home(json!([super::super::onhome::owner_tile("climate.living_room_thermostat"), bedjet]));
        into_smart_card(&mut h);
        let at = go_to_item(&mut h, "climate.master_bedroom_bedjet", 2100) + 100;
        let out = press(&mut h, "ok", at);
        assert!(act_of(&out).is_none(), "opening it asks nothing");
        assert_eq!((h.layer, h.sheet.is_none(), h.state()["onhome"]["sheetKind"].clone()), (Layer::OnHome, true, json!("thermostat")), "its own sheet, not the rows of the controls");
        // Right is a step up: nothing goes while it is turned, and one call goes when it has rested (the helper makes it `climate.set_temperature`)
        assert!(act_of(&press(&mut h, "right", at + 100)).is_none());
        assert!(act_of(&press(&mut h, "right", at + 150)).is_none());
        assert!(h.due_at().is_some_and(|d| d <= at + 150 + 300));
        let out = h.update(&Input::Tick, at + 460);
        assert_eq!(act_of(&out), Some(json!({"id": "climate.master_bedroom_bedjet", "verb": "temperature", "value": 74.0})), "two steps up from 72");
        assert!(act_of(&h.update(&Input::Tick, at + 900)).is_none(), "once");
        // Down to the modes: walking to Cool changes nothing, OK asks for it
        press(&mut h, "down", at + 1000);
        assert!(act_of(&press(&mut h, "right", at + 1100)).is_none());
        let out = press(&mut h, "ok", at + 1200);
        assert_eq!(act_of(&out), Some(json!({"id": "climate.master_bedroom_bedjet", "verb": "hvac_mode", "value": "cool"})));
        // Back, and the arrows take the card to the living room's, which holds a range (one end moves, and both go in the call). The OK that chose
        // the mode pressed nothing under the sheet: this OK opens the item in front now, not the one the sheet was opened on
        press(&mut h, "back", at + 1300);
        assert_eq!((h.layer, h.thermostat.is_none()), (Layer::None, true));
        let at = go_to_item(&mut h, "climate.living_room_thermostat", at + 1400) + 100;
        press(&mut h, "ok", at);
        assert_eq!(h.thermostat.as_ref().map(|t| (t.id.as_str(), t.rows.clone())), Some(("climate.living_room_thermostat", vec![ThermostatRow::Low, ThermostatRow::High, ThermostatRow::Mode])), "a low, a high and the modes");
        press(&mut h, "left", at + 100);
        let out = h.update(&Input::Tick, at + 500);
        assert_eq!(act_of(&out), Some(json!({"id": "climate.living_room_thermostat", "verb": "range", "value": {"low": 66.0, "high": 76.0}})));
        // Home Assistant said no: the card says so in its words, over the sheet
        h.update(&Input::Fetched { tag: ONHOME_TAG, status: 409, data: json!({"ok": false, "error": {"code": "refused", "message": "Home Assistant does not let the TV do that"}}) }, at + 600);
        assert_eq!(h.onhome_notice(), Some("Home Assistant does not let the TV do that"));
    }

    #[test]
    fn the_pointer_steps_a_set_point_and_picks_a_mode_on_the_thermostat_sheet_and_a_click_beside_it_puts_it_away() {
        use super::super::onhome::thermostat_surface::{hit, layout};
        let mut bedjet = super::super::onhome::owner_tile("climate.master_bedroom_bedjet");
        bedjet["ctl"]["mode"] = json!("heat");
        let mut h = smart_home(json!([bedjet]));
        into_smart_card(&mut h);
        press(&mut h, "ok", 2100);
        let t = h.thermostat.as_ref().expect("the sheet").clone();
        let chars = t.modes.iter().map(|m| super::super::onhome::thermostat::ThermostatSheet::mode_word(m).chars().count()).max().unwrap_or(0);
        let lay = layout(&h.onhome_metrics, &t.shown(), t.modes.len(), chars);
        let centre = |f: ui::geometry::Frame| (lay.pane.x + f.x + f.w / 2.0, lay.pane.y + f.y + f.h / 2.0);
        let click = |h: &mut Home, (x, y): (f32, f32), now: u64| {
            h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, now);
            let out = h.update(&Input::Pointer { x, y, buttons: 1, visible: true }, now + 10);
            let mut up = h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, now + 20);
            up.extend(out);
            up
        };
        assert!(hit(&lay, &t, centre(lay.sets[0].plus).0, centre(lay.sets[0].plus).1).is_some());
        click(&mut h, centre(lay.sets[0].plus), 3000);
        let out = h.update(&Input::Tick, 3400);
        assert_eq!(act_of(&out).map(|b| (b["verb"].clone(), b["value"].clone())), Some((json!("temperature"), json!(73.0))), "a click on the plus is a step up");
        let out = click(&mut h, centre(lay.chips[2]), 4000);
        assert_eq!(act_of(&out).map(|b| (b["verb"].clone(), b["value"].clone())), Some((json!("hvac_mode"), json!("cool"))), "a click on a chip chooses the mode");
        click(&mut h, (5.0, 5.0), 5000);
        assert_eq!((h.layer, h.thermostat.is_none()), (Layer::None, true), "a click beside the sheet is Back");
    }

    #[test]
    fn a_turn_of_the_wheel_scrolls_the_card_under_the_pointer_a_stride_and_nothing_else() {
        let tiles: Vec<Value> = (0..16).map(|i| json!({"id": format!("light.l{i}"), "name": format!("Lamp {i}"), "group": "lights", "words": "Off", "tone": "off", "icon": "lightbulb", "available": true, "can": ["toggle"], "verbs": ["turn_on", "turn_off", "toggle"], "ctl": {"on": false}})).collect();
        let mut h = smart_home(Value::Array(tiles));
        let card = h.smart_of("smart").and_then(|s| s.screen_rect(0)).expect("the first item");
        let max = h.smart_of("smart").map(|s| s.plan().max_offset()).unwrap();
        assert!(max > 0 && h.wheel_scrolls(), "more items than the card holds: it scrolls");
        let wheel = |h: &mut Home, name: &str, at: u64| {
            h.update(&Input::Key { name: name.into(), down: true, repeat: false }, at);
            h.update(&Input::Key { name: name.into(), down: false, repeat: false }, at + 10);
        };
        // the pointer is elsewhere: the card does not move
        h.update(&Input::Pointer { x: 1500.0, y: 200.0, buttons: 0, visible: true }, 2000);
        wheel(&mut h, "wheeldown", 2100);
        assert_eq!(h.smart_of("smart").map(Smart::offset), Some(0));
        // over the card: a stride a notch, never beyond the ends
        h.update(&Input::Pointer { x: card.x + 10.0, y: card.y + 10.0, buttons: 0, visible: true }, 2200);
        wheel(&mut h, "wheeldown", 2300);
        assert_eq!(h.smart_of("smart").map(Smart::offset), Some(1));
        for i in 0..20 {
            wheel(&mut h, "wheeldown", 2400 + i * 20);
        }
        assert_eq!(h.smart_of("smart").map(Smart::offset), Some(max));
        wheel(&mut h, "wheelup", 3000);
        assert_eq!(h.smart_of("smart").map(Smart::offset), Some(max - 1));
        for i in 0..20 {
            wheel(&mut h, "wheelup", 3100 + i * 20);
        }
        assert_eq!(h.smart_of("smart").map(Smart::offset), Some(0));
        assert_eq!((h.layer, item_in_front(&h)), (Layer::None, None), "the wheel moves the view, not the focus");
    }

    #[test]
    fn the_layout_editor_arranges_the_items_of_a_smart_home_card_with_its_items_button_and_never_switches_one() {
        use super::super::editor::Zone;
        let mut h = smart_home(json!([lamp(), number_sensor()]));
        let mut out = Vec::new();
        h.edit_home("smart", &mut out);
        let ed = h.editor.as_mut().expect("the editor");
        assert!(ed.arrangeable(), "a smart-home card has items to arrange");
        let items = ed.buttons().iter().position(|b| b.id == "items").expect("the Items button");
        assert!(ed.buttons()[items].enabled);
        // the bar's button: OK on it hands the keys to the card
        ed.zone = Zone::Bar;
        ed.bar_index = items;
        press(&mut h, "ok", 2000);
        assert_eq!(h.editor.as_ref().map(|e| e.zone), Some(Zone::Items));
        assert!(h.items_editing("smart"), "the card is drawn entered");
        assert_eq!(h.smart_of("smart").and_then(Smart::focused), Some(0));
        // the arrows choose an item; OK opens its menu and does not switch it
        let out = press(&mut h, "right", 2100);
        assert!(act_of(&out).is_none());
        let chosen = h.smart_of("smart").and_then(Smart::focused);
        assert!(chosen.is_some());
        let out = press(&mut h, "ok", 2200);
        assert!(act_of(&out).is_none(), "OK here opens the menu: it is not the card's own use of OK");
        assert!(h.smart_of("smart").is_some_and(|s| s.arrange.is_some()));
        // Right picks the next size and OK keeps it: the owner's change goes to the helper
        press(&mut h, "right", 2300);
        let out = press(&mut h, "ok", 2400);
        assert!(out.iter().any(|e| matches!(e, Effect::Request { path, body: Some(b), .. } if path == "/v1/onhome/picks" && b.get("size").is_some())), "{out:?}");
        assert!(h.smart_of("smart").is_some_and(|s| s.arrange.is_none()));
        // Back leaves the items for the bar, on the button that brought it here; Back from the bar asks nothing (nothing changed on the layout)
        press(&mut h, "back", 2500);
        assert_eq!(h.editor.as_ref().map(|e| e.zone), Some(Zone::Bar));
        assert!(!h.items_editing("smart"));
        // a card that is not a smart-home card has no items to arrange
        let ed = h.editor.as_mut().unwrap();
        ed.sel = "weather".into();
        assert!(!ed.arrangeable() && !ed.buttons().iter().find(|b| b.id == "items").unwrap().enabled);
    }

    fn lock_tile(state: &str, can: &str) -> Value {
        json!({"id": "lock.front", "name": "Front door", "group": "locks", "words": state, "tone": "ok", "icon": "lock", "available": true, "can": [can], "verbs": ["lock"], "sensitive": ["unlock"], "ctl": {"state": state}})
    }

    fn requests_to(out: &[Effect], path: &str) -> Vec<Value> {
        out.iter().filter_map(|e| match e {
            Effect::Request { path: p, body, .. } if p == path => body.clone(),
            _ => None,
        }).collect()
    }

    #[test]
    fn an_open_lock_is_locked_by_one_ok_and_a_locked_one_asks_first_and_unlocks_only_after_the_answer() {
        // open: OK locks at once, through the ordinary route
        let mut h = smart_home(json!([lock_tile("unlocked", "lock")]));
        into_smart_card(&mut h);
        let out = press(&mut h, "ok", 2100);
        assert_eq!(act_of(&out), Some(json!({"id": "lock.front", "verb": "lock", "value": null})));
        assert_eq!(h.layer, Layer::None);
        // locked: OK opens the question, nothing is sent
        let mut h = smart_home(json!([lock_tile("locked", "unlock")]));
        into_smart_card(&mut h);
        let out = press(&mut h, "ok", 2100);
        assert!(act_of(&out).is_none() && requests_to(&out, "/v1/onhome/unlock").is_empty());
        assert_eq!((h.layer, h.sheet.as_ref().map(|s| s.asking)), (Layer::OnHome, Some(true)));
        // Cancel in front: OK on it does nothing and shows the sheet's rows; the question can be asked again
        let out = press(&mut h, "ok", 2200);
        assert!(out.iter().all(|e| !matches!(e, Effect::Read { .. } | Effect::Request { .. })));
        assert_eq!(h.sheet.as_ref().map(|s| s.asking), Some(false));
        press(&mut h, "ok", 2300);
        assert_eq!(h.sheet.as_ref().map(|s| s.asking), Some(true));
        // Back leaves the question, and the sheet
        press(&mut h, "back", 2400);
        assert_eq!((h.layer, h.sheet.as_ref().map(|s| s.asking)), (Layer::OnHome, Some(false)));
        press(&mut h, "ok", 2500);
        // down to Unlock, OK: the token is read, and only then is the unlock asked, once, with that token
        press(&mut h, "down", 2600);
        let out = press(&mut h, "ok", 2700);
        assert!(out.iter().any(|e| matches!(e, Effect::Read { tag, path } if *tag == UNLOCK_TAG && path == "onhome/shell.json")));
        assert!(requests_to(&out, "/v1/onhome/unlock").is_empty(), "nothing is asked before the token is read");
        assert!(act_of(&out).is_none(), "an unlock is never an ordinary act");
        assert_eq!(h.layer, Layer::None);
        let out = h.update(&Input::Fetched { tag: UNLOCK_TAG, status: 200, data: json!({"token": "abc"}) }, 2800);
        assert_eq!(requests_to(&out, "/v1/onhome/unlock"), vec![json!({"id": "lock.front", "token": "abc"})]);
        // a token that cannot be read: said, nothing asked
        let mut h = smart_home(json!([lock_tile("locked", "unlock")]));
        into_smart_card(&mut h);
        press(&mut h, "ok", 2100);
        press(&mut h, "down", 2200);
        press(&mut h, "ok", 2300);
        let out = h.update(&Input::Fetched { tag: UNLOCK_TAG, status: 404, data: Value::Null }, 2400);
        assert!(requests_to(&out, "/v1/onhome/unlock").is_empty());
        assert_eq!(h.onhome_notice(), Some("The TV cannot unlock from here"));
        // a token that arrives with nothing asked for is nothing
        let out = h.update(&Input::Fetched { tag: UNLOCK_TAG, status: 200, data: json!({"token": "abc"}) }, 2500);
        assert!(requests_to(&out, "/v1/onhome/unlock").is_empty());
    }

    #[test]
    fn no_sequence_of_keys_on_the_card_or_a_sheet_sends_an_unlock_through_the_ordinary_route() {
        for state in ["locked", "unlocked"] {
            let mut h = smart_home(json!([lock_tile(state, if state == "locked" { "unlock" } else { "lock" }), lamp()]));
            into_smart_card(&mut h);
            for (i, k) in ["ok", "down", "ok", "up", "left", "right", "ok", "down", "down", "ok", "back", "menu", "ok", "ok", "back", "back"].iter().enumerate() {
                let out = press(&mut h, k, 2100 + i as u64 * 100);
                if let Some(body) = act_of(&out) {
                    assert!(!matches!(body["verb"].as_str(), Some("unlock" | "open")), "{state} {k}: {body}");
                }
            }
        }
    }

    #[test]
    fn a_refusal_comes_back_in_words_on_the_card_and_the_sheet_and_goes_by_itself() {
        let mut h = smart_home(json!([lamp()]));
        into_smart_card(&mut h);
        press(&mut h, "ok", 2100);
        let refused = json!({"ok": false, "error": {"code": "unavailable", "message": "Home Assistant cannot reach it right now"}});
        h.update(&Input::Fetched { tag: ONHOME_TAG, status: 409, data: refused }, 2200);
        assert_eq!(h.onhome_notice(), Some("Home Assistant cannot reach it right now"));
        h.update(&Input::Tick, 6300);
        assert_eq!(h.onhome_notice(), None);
        // a 200 says nothing
        h.update(&Input::Fetched { tag: ONHOME_TAG, status: 200, data: json!({"ok": true}) }, 6400);
        assert_eq!(h.onhome_notice(), None);
    }

    #[test]
    fn a_sheet_closes_when_its_thing_leaves_home_and_a_card_with_no_tile_opens_the_settings_page() {
        let mut h = smart_home(json!([thermostat()]));
        into_smart_card(&mut h);
        press(&mut h, "ok", 2100);
        assert_eq!(h.layer, Layer::OnHome);
        h.update(&Input::Helper { topic: "onhome".into(), data: json!({"state": "connected", "tiles": []}) }, 2200);
        assert_eq!(h.layer, Layer::None, "the thing is gone from Home: so is its sheet");
        // with no tile the card has no inner layer: OK opens the page
        let mut h = smart_home(json!([]));
        h.update(&Input::Helper { topic: "onhome".into(), data: json!({"state": "unpaired", "tiles": []}) }, 1100);
        h.walk_to_card("smart", 1200);
        press(&mut h, "ok", 2100);
        assert!(h.inner.is_none());
        assert_eq!(h.take_requests(), vec![Request::SettingsPage("onhome".into())]);
    }

    fn lane(h: &Home) -> (&'static str, &'static str, &'static str) {
        let l = h.ha_lane();
        (l.title, l.sub, l.role)
    }

    #[test]
    fn the_at_home_card_with_no_chip_says_what_home_assistants_page_says_from_the_same_roster_and_lease() {
        let mut h = home();
        let helper = |h: &mut Home, topic: &str, data: Value| {
            h.update(&Input::Helper { topic: topic.into(), data }, 1100);
        };
        helper(&mut h, "plugins", roster(json!([])));
        assert_eq!(lane(&h), ("Home Assistant", "Not switched on", "muted"), "the TV's own answer: no owner plugins");
        let plugin = |state: &str| json!([{"id": "home-assistant", "name": "Home Assistant", "state": state}]);
        helper(&mut h, "plugins", roster(plugin("inert")));
        assert_eq!(lane(&h), ("Home Assistant", "Cannot run", "warning"));
        helper(&mut h, "plugins", roster(json!([{"id": "home-assistant", "state": "waiting", "reason": "crashed"}])));
        assert_eq!(lane(&h), ("Home Assistant", "Not running", "warning"));
        helper(&mut h, "plugins", roster(plugin("connected")));
        assert_eq!(lane(&h), ("Home Assistant", "Not linked", "muted"), "no lease topic yet");
        helper(&mut h, "ha.link", json!({"state": "auth_failed"}));
        assert_eq!(lane(&h), ("Home Assistant", "Token refused", "danger"));
        helper(&mut h, "ha.link", json!({"state": "offline"}));
        assert_eq!(lane(&h), ("Home Assistant", "Offline", "warning"));
        helper(&mut h, "ha.link", json!({"state": "connected"}));
        assert_eq!(lane(&h), ("Home Assistant", "Nothing to show", "muted"));
        assert!(h.ha_items().is_empty());
    }

    #[test]
    fn the_at_home_chips_are_every_connected_plugins_cards_but_the_ones_home_draws_itself_the_most_urgent_first_and_a_tap_goes_to_the_plugin_that_owns_it() {
        let mut h = home();
        h.update(&Input::Helper { topic: "plugins".into(), data: roster(json!([{"id": "home-assistant", "state": "connected"}, {"id": "lock", "state": "waiting"}])) }, 1100);
        let card = |id: &str, priority: &str, action: Option<&str>| {
            let actions = action.map(|a| json!([{"id": "a", "label": "Do", "action": a, "target": "movie"}])).unwrap_or(json!([]));
            json!({"id": id, "revision": 1, "card": {"size": "half", "priority": priority, "content": {"type": "facts", "title": id, "role": "accent", "facts": []}, "actions": actions}})
        };
        h.update(&Input::Helper { topic: "plugin.home-assistant.cards".into(), data: json!({"items": [card("room", "new", None), card("doorbell", "ringing", Some("answer"))]}) }, 1101);
        h.update(&Input::Helper { topic: "plugin.weather.cards".into(), data: json!({"items": [card("now", "new", None)]}) }, 1102);
        h.update(&Input::Helper { topic: "plugin.lock.cards".into(), data: json!({"items": [card("front", "new", None)]}) }, 1103);
        let ids: Vec<(String, String)> = h.ha_items().iter().map(|c| (c["pluginId"].as_str().unwrap().into(), c["id"].as_str().unwrap().into())).collect();
        assert_eq!(ids, vec![("home-assistant".to_string(), "doorbell".to_string()), ("home-assistant".to_string(), "room".to_string())], "ringing first; weather is Home's own card; lock is not connected");
        let wid = h.cards().iter().find(|e| e.kind == "homeassistant").map(|e| e.id.clone()).expect("the card");
        let mut out = Vec::new();
        h.inner_index = Some(0);
        assert!(h.inner_key(&wid, "ok", &mut out));
        assert_eq!(out, vec![Effect::Post { path: "/v1/plugins/home-assistant/actions/answer".into(), body: json!({"target": "movie"}) }]);
    }

    #[test]
    fn ok_on_the_at_home_card_with_no_chip_opens_home_assistants_page() {
        let mut h = home();
        let wid = h.cards().iter().find(|e| e.kind == "homeassistant").map(|e| e.id.clone()).expect("the card");
        assert!(!h.has_inner(&wid), "no chip, no inner layer");
        let mut out = Vec::new();
        h.activate_card(&wid, &mut out);
        assert_eq!(h.take_requests(), vec![Request::SettingsPage("homeassistant".into())]);
    }

    #[test]
    fn camera_stills_pause_when_home_is_hidden_asleep_or_has_a_sheet() {
        let mut h = home();
        h.cameras = json!({"state": "ready"});
        assert!(h.camera_stills_visible());
        h.layer = Layer::Apps;
        assert!(!h.camera_stills_visible());
        h.layer = Layer::None;
        h.update(&Input::Topic { topic: "screen".into(), data: json!({"on": false}) }, 2000);
        assert!(!h.camera_stills_visible());
        h.update(&Input::Topic { topic: "screen".into(), data: json!({"on": true}) }, 2100);
        assert!(h.camera_stills_visible());
        h.active = false;
        assert!(!h.camera_stills_visible());
        h.active = true;
        h.cameras = json!({"state": "offline"});
        assert!(!h.camera_stills_visible());
    }

    #[test]
    fn camera_stills_pause_while_something_is_over_home_and_come_again_when_it_is_calm_no_more() {
        let mut h = home();
        h.cameras = json!({"state": "ready"});
        h.stills.configure(&[("front".into(), 100, 56, true)]);
        assert!(h.stills.due_at(h.camera_stills_visible()).is_some(), "a still is asked for");
        h.set_calm(true);
        assert!(!h.camera_stills_visible());
        assert_eq!(h.stills.due_at(h.camera_stills_visible()), None, "nothing is asked for while a panel or Settings is over it");
        h.set_calm(false);
        assert!(h.camera_stills_visible());
        assert!(h.stills.due_at(h.camera_stills_visible()).is_some(), "and the stills come again");
    }

    #[test]
    fn the_shipped_layout_is_drawn_where_it_always_was() {
        let h = home();
        assert_eq!(
            h.state()["cards"],
            json!([["clock", 64, 134, 584, 222], ["home", 64, 376, 584, 238], ["news", 668, 134, 735, 480], ["weather", 1423, 134, 433, 480]])
        );
        assert_eq!(h.dock_rect(), Some(Rect { x: 64, y: 634, w: 1792, h: 392 }));
    }

    #[test]
    fn the_focus_walks_the_status_bar_the_cards_and_the_dock() {
        let mut h = home();
        assert_eq!(h.state()["focusLayer"], "statusbar");
        press(&mut h, "down", 2000);
        assert_eq!(h.state()["widgetFocusId"], "news", "down from the bar's first button: the card under it, the bar being in the middle of the screen");
        press(&mut h, "down", 2200);
        assert_eq!(h.state()["focusLayer"], "dock");
        assert!(h.dock.index > 0, "the tile under the news card, not the first");
        press(&mut h, "left", 2300);
        press(&mut h, "left", 2350);
        press(&mut h, "left", 2400);
        press(&mut h, "left", 2450);
        press(&mut h, "left", 2500);
        assert_eq!(h.dock.index, 0, "the row's end holds");
        press(&mut h, "up", 2600);
        assert_eq!(h.state()["widgetFocusId"], "news", "up into the cards lands on the card remembered there");
        press(&mut h, "left", 2650);
        assert_eq!(h.state()["widgetFocusId"], "home", "left from the news card: the home card's centre is nearer the news card's");
        press(&mut h, "down", 2750);
        assert_eq!(h.state()["focusLayer"], "dock");
        press(&mut h, "back", 2780);
        assert_eq!(h.state()["widgetFocusId"], "home", "back from the dock goes to the lowest card by x");
        press(&mut h, "up", 2800);
        assert_eq!(h.state()["widgetFocusId"], "clock");
        press(&mut h, "up", 2850);
        assert_eq!(h.state()["focusLayer"], "statusbar");
    }

    #[test]
    fn right_at_the_end_of_a_dock_row_rolls_over_to_the_next_row_and_left_at_its_start_comes_back() {
        let mut h = home();
        let mut lps = vec![json!({"id": APPS_TILE, "title": "Apps", "unmovable": true, "icon": "/a.png"})];
        lps.extend((1..30).map(|n| json!({"id": format!("app{n}"), "title": format!("App {n}"), "icon": "/a.png", "iconColor": "#101010"})));
        h.update(&Input::Luna { tag: LP_TAG, payload: json!({"launchPoints": lps}) }, 1100);
        let cols = h.dock.cols();
        assert_eq!((cols, h.dock.points.len(), h.dock.visible_rows()), (12, 30, 2));
        let at = |h: &mut Home, i: usize| {
            h.dock.focus(i);
            h.register_dock();
            h.focus.focus(Id(DOCK_ID + i as u32));
        };
        at(&mut h, cols - 1);
        press(&mut h, "right", 2000);
        assert_eq!((h.dock.index, h.focus.current()), (cols, Some(Id(DOCK_ID + cols as u32))), "the next row's first tile");
        press(&mut h, "left", 2100);
        assert_eq!((h.dock.index, h.focus.current()), (cols - 1, Some(Id(DOCK_ID + cols as u32 - 1))), "the row above's last tile");
        press(&mut h, "left", 2200);
        assert_eq!(h.dock.index, cols - 2, "inside a row the arrows are as before");
        at(&mut h, 0);
        press(&mut h, "left", 2300);
        assert_eq!(h.dock.index, 0, "the start of the dock holds");
        at(&mut h, 29);
        press(&mut h, "right", 2400);
        assert_eq!(h.dock.index, 29, "the end of the dock holds");
        // onto a row that is below the two shown: the dock scrolls to it and the tile is registered inside the card
        at(&mut h, 2 * cols - 1);
        press(&mut h, "right", 2500);
        let (card, tile) = (h.dock_rect().unwrap(), h.focus.frame(Id(DOCK_ID + 2 * cols as u32)).unwrap());
        assert_eq!((h.dock.index, h.dock.scroll_row), (2 * cols, 1));
        assert!(tile.y >= card.y as f32 && tile.bottom() <= (card.y + card.h) as f32, "{tile:?} in {card:?}");
    }

    #[test]
    fn ok_on_a_tile_launches_and_a_digit_opens_a_tile_and_apps_opens_the_library() {
        let mut h = home();
        to_the_dock_under_the_home_card(&mut h, 2000);
        let out = press(&mut h, "ok", 2400);
        assert!(out.iter().any(|e| matches!(e, Effect::Luna { uri, payload, .. } if uri.ends_with("/launch") && payload["id"] == "netflix")));
        let out = press(&mut h, "3", 2600);
        assert!(out.iter().any(|e| matches!(e, Effect::Luna { payload, .. } if payload["id"] == "youtube")));
        assert_eq!(h.dock.index, 2);
        press(&mut h, "1", 2700);
        assert_eq!(h.layer, Layer::Apps);
        press(&mut h, "back", 2800);
        assert_eq!(h.layer, Layer::None);
    }

    #[test]
    fn a_held_ok_on_a_card_opens_the_editor_and_its_release_is_not_an_ok() {
        let mut h = home();
        press(&mut h, "down", 2000);
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 3000);
        assert!(h.editor.is_none());
        h.update(&Input::Tick, 3800);
        assert_eq!(h.layer, Layer::Edit);
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: true }, 3850);
        h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 3900);
        assert_eq!(h.editor.as_ref().map(|e| e.mode_name()), Some("select"), "the release of the hold picked the card up");
        assert_eq!(h.editor.as_ref().map(|e| e.sel.as_str()), Some("news"));
    }

    #[test]
    fn a_held_ok_on_a_tile_opens_its_options_and_move_picks_it_up() {
        let mut h = home();
        press(&mut h, "2", 1500);
        h.close_all();
        assert_eq!(h.dock.index, 1);
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 3000);
        h.update(&Input::Tick, 3800);
        assert_eq!(h.layer, Layer::Menu);
        assert_eq!(h.menu.iter().map(|r| r.id).collect::<Vec<_>>(), vec!["open", "move", "hide-dock", "rename", "uninstall"]);
        h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 3900);
        assert_eq!(h.layer, Layer::Menu, "the release of the hold is not an OK");
        press(&mut h, "down", 4000);
        press(&mut h, "ok", 4100);
        assert!(h.dock.moving);
        press(&mut h, "right", 4200);
        assert_eq!(h.dock.ids()[2], "netflix");
        let out = press(&mut h, "ok", 4300);
        assert!(out.iter().any(|e| matches!(e, Effect::Write { path, text } if path == "theme/theme.json" && text.contains("dockOrder"))));
        assert!(!h.dock.moving);
    }

    fn removals(out: &[Effect]) -> usize {
        out.iter().filter(|e| matches!(e, Effect::Luna { uri, .. } if uri.ends_with("/remove"))).count()
    }

    /// Home with the tile menu of the second tile (Netflix) up and its Uninstall row in front.
    fn menu_on_uninstall() -> Home {
        let mut h = home();
        press(&mut h, "2", 1500);
        h.close_all();
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 3000);
        h.update(&Input::Tick, 3800);
        h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 3900);
        assert_eq!(h.layer, Layer::Menu);
        for k in 0..4 {
            press(&mut h, "down", 4000 + k * 100);
        }
        assert_eq!(h.menu[h.menu_index].id, "uninstall");
        h
    }

    #[test]
    fn uninstall_asks_first_keep_it_has_the_focus_and_back_or_ok_on_it_removes_nothing() {
        let mut h = menu_on_uninstall();
        let out = press(&mut h, "ok", 5000);
        assert_eq!((h.layer, removals(&out)), (Layer::Confirm, 0), "OK on Uninstall asks, it does not remove");
        assert_eq!(h.confirm.as_ref().map(|c| (c.app_id.as_str(), c.index)), Some(("netflix", 0)), "Keep it is in front");
        // the surface reports the answers' places: the safe one has the focus
        h.set_confirm_frames(&[ui::geometry::Frame::new(700.0, 600.0, 160.0, 64.0), ui::geometry::Frame::new(880.0, 600.0, 200.0, 64.0)]);
        assert_eq!(h.focus.current(), Some(Id(CONFIRM_ID)));
        let out = press(&mut h, "ok", 5200);
        assert_eq!((h.layer, removals(&out)), (Layer::None, 0), "OK on the default is Keep it");
        assert!(h.confirm.is_none());
        // Back is Keep it as well
        let mut h = menu_on_uninstall();
        press(&mut h, "ok", 5000);
        let out = press(&mut h, "back", 5200);
        assert_eq!((h.layer, removals(&out)), (Layer::None, 0));
    }

    #[test]
    fn only_ok_on_uninstall_after_moving_to_it_removes_and_once() {
        let mut h = menu_on_uninstall();
        press(&mut h, "ok", 5000);
        h.set_confirm_frames(&[ui::geometry::Frame::new(700.0, 600.0, 160.0, 64.0), ui::geometry::Frame::new(880.0, 600.0, 200.0, 64.0)]);
        let mut out = press(&mut h, "right", 5100);
        assert_eq!((h.layer, removals(&out)), (Layer::Confirm, 0), "moving to Uninstall removes nothing");
        assert_eq!(h.focus.current(), Some(Id(CONFIRM_ID + 1)));
        // the press and the release of one OK is one answer
        out = h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 5200);
        assert_eq!(removals(&out), 0, "a press alone answers nothing");
        out = h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 5240);
        assert_eq!((h.layer, removals(&out)), (Layer::None, 1));
        assert!(out.iter().any(|e| matches!(e, Effect::Luna { payload, .. } if payload["id"] == "netflix")));
        let again = h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 5300);
        assert_eq!(removals(&again), 0, "a release with no press is nothing");
    }

    #[test]
    fn a_click_on_an_answer_gives_it_and_only_apps_lg_lets_go_that_are_not_protected_have_the_row() {
        let mut h = menu_on_uninstall();
        press(&mut h, "ok", 5000);
        let (keep, remove) = (ui::geometry::Frame::new(700.0, 600.0, 160.0, 64.0), ui::geometry::Frame::new(880.0, 600.0, 200.0, 64.0));
        h.set_confirm_frames(&[keep, remove]);
        let at = |f: &ui::geometry::Frame| (f.x + 10.0, f.y + 10.0);
        let click = |h: &mut Home, (x, y): (f32, f32), t: u64| {
            let mut all = Vec::new();
            for (b, dt) in [(0, 0), (1, 20), (0, 40)] {
                all.extend(h.update(&Input::Pointer { x, y, buttons: b, visible: true }, t + dt));
            }
            all
        };
        let out = click(&mut h, at(&keep), 5100);
        assert_eq!((h.layer, removals(&out)), (Layer::None, 0), "a click on Keep it keeps it");
        let mut h = menu_on_uninstall();
        press(&mut h, "ok", 5000);
        h.set_confirm_frames(&[keep, remove]);
        let out = click(&mut h, at(&remove), 5100);
        assert_eq!((h.layer, removals(&out)), (Layer::None, 1));
        // the row is only for what LG lets go and is not LG's, LumaTV's or the Homebrew Channel's
        let rows = |lp: Value| Home::menu_rows(&json!({"id": lp["id"], "removable": super::super::dock::removable_app(&lp), "onDock": true}), "dock").iter().any(|r| r.id == "uninstall");
        assert!(rows(json!({"id": "org.webosbrew.hyperhdr.loader", "removable": true})));
        assert!(!rows(json!({"id": "org.nphil.lumatv", "removable": true})));
        assert!(!rows(json!({"id": "org.nphil.lumatv.card", "removable": true})));
        assert!(!rows(json!({"id": "org.webosbrew.hbchannel", "removable": true})));
        assert!(!rows(json!({"id": "com.webos.app.browser", "removable": true})));
        assert!(!rows(json!({"id": "netflix", "removable": true, "systemApp": true})));
        assert!(!rows(json!({"id": "netflix", "removable": false})));
        assert!(!rows(json!({"id": "netflix"})));
    }

    #[test]
    fn the_files_are_read_every_two_seconds_and_a_new_layout_is_applied() {
        let mut h = home();
        h.update(&Input::Fetched { tag: THEME_TAG, status: 404, data: Value::Null }, 1100);
        h.update(&Input::Fetched { tag: LAYOUT_TAG, status: 404, data: Value::Null }, 1100);
        let out = h.update(&Input::Tick, 3100);
        assert!(out.iter().any(|e| matches!(e, Effect::Read { tag, .. } if *tag == LAYOUT_TAG)));
        let user = json!({"grid": {"rows": 16}, "widgets": [{"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 6, "rows": 4},
            {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]});
        h.update(&Input::Fetched { tag: THEME_TAG, status: 404, data: Value::Null }, 3200);
        let out = h.update(&Input::Fetched { tag: LAYOUT_TAG, status: 200, data: user }, 3200);
        assert!(out.contains(&Effect::Redraw));
        assert_eq!(h.cards().len(), 1, "the owner's list of cards replaces the shipped one whole");
        assert_eq!(h.cards()[0].span, 6);
    }

    #[test]
    fn the_chip_is_laid_out_on_the_names_measured_width_and_its_avatar_is_set_in_the_left_cap() {
        let c = Chip::new(48.0, Some(50.0));
        assert_eq!((c.inset, c.avatar, c.name_x, c.width), (4.0, 40.0, 54.0, 54.0 + 50.0 + 17.0), "an even 4 px round a 40 px disc in a 48 px pill");
        assert_eq!(c.inset * 2.0 + c.avatar, 48.0);
        let big = Chip::new(108.0, Some(110.0));
        assert_eq!((big.inset, big.avatar), (9.0, 90.0), "the inset follows the pill's height");
        assert_eq!(Chip::new(48.0, None).width, 48.0, "no name: the avatar alone, a round button");
        let mut h = home();
        h.update(&Input::Helper { topic: "profiles".into(), data: json!({"ready": true, "active": "nitin", "owner": "nitin", "canAdmin": true, "switchedAt": 1,
            "profiles": [{"id": "nitin", "name": "Nitin", "initial": "N", "color": "#A79BF5", "active": true, "locked": false}]}) }, 1500);
        assert_eq!(h.name_to_measure().as_deref(), Some("Nitin"), "the surface is asked for the width");
        h.set_name_width("Nitin".into(), 50.0);
        assert_eq!(h.name_to_measure(), None);
        let widths: Vec<f32> = h.status_frames().iter().map(|f| f.2).collect();
        assert_eq!(widths.last().copied(), Some(Chip::new(48.0, Some(50.0)).width), "the chip is the last button, as wide as its parts");
        h.set_text_scale(2.0);
        assert_eq!(h.name_to_measure().as_deref(), Some("Nitin"), "a text size has its own width");
    }

    fn named(h: &mut Home) {
        h.update(&Input::Helper { topic: "profiles".into(), data: json!({"ready": true, "active": "nitin", "owner": "nitin", "canAdmin": true, "switchedAt": 1,
            "profiles": [{"id": "nitin", "name": "Nitin", "initial": "N", "color": "#A79BF5", "active": true, "locked": false}]}) }, 1500);
    }

    #[test]
    fn the_bar_reads_actions_then_who_is_watching_and_the_dpad_walks_it_in_that_order() {
        let mut h = home();
        let kinds: Vec<String> = h.status_items().iter().map(|it| kind_of(it).to_owned()).collect();
        assert_eq!(kinds, ["search", "cameras", "notifications", "settings", "profile"], "finding and watching first, the bell and the gear after, the chip last");
        let purposes: Vec<Purpose> = kinds.iter().map(|k| Purpose::of(k)).collect();
        assert_eq!(purposes, [Purpose::Action, Purpose::Action, Purpose::Action, Purpose::Action, Purpose::Identity]);
        let frames = h.status_frames();
        assert!(frames.windows(2).all(|w| w[0].0 + w[0].2 < w[1].0), "left to right, no overlap: {frames:?}");
        // the focus starts on the first and Right steps along the bar in the order it is drawn; Left comes back
        assert_eq!(h.focus.current(), Some(Id(STATUS_ID)));
        for i in 1..5u32 {
            press(&mut h, "right", 2000 + u64::from(i) * 100);
            assert_eq!(h.focus.current(), Some(Id(STATUS_ID + i)), "Right {i}");
        }
        press(&mut h, "right", 2600);
        assert_eq!(h.focus.current(), Some(Id(STATUS_ID + 4)), "the end of the bar holds");
        for i in (0..4u32).rev() {
            press(&mut h, "left", 3000 + u64::from(i) * 100);
            assert_eq!(h.focus.current(), Some(Id(STATUS_ID + i)), "Left to {i}");
        }
    }

    #[test]
    fn the_gaps_are_the_themes_spacing_inside_a_group_and_between_groups_and_the_pill_hugs_them() {
        let mut h = home();
        named(&mut h);
        h.set_name_width("Nitin".into(), 47.63);
        let frames = h.status_frames();
        let pill = h.status_pill();
        let (within, between) = (h.status_gaps.0, h.status_gaps.1);
        assert_eq!((within, between), (8.0, 20.0), "space(2) and space(5) of the shipped theme");
        for i in 0..3 {
            assert_eq!(frames[i + 1].0 - (frames[i].0 + frames[i].2), within, "buttons {i} and {}", i + 1);
        }
        assert_eq!(frames[4].0 - (frames[3].0 + frames[3].2), between, "the chip stands apart from the actions");
        assert_eq!((frames[0].0 - pill.x, pill.x + pill.w - (frames[4].0 + frames[4].2)), (STATUS_INSET, STATUS_INSET), "concentric with the pill's caps");
        assert!((pill.x + pill.w / 2.0 - 960.0).abs() <= 0.5, "the pill is in the middle of the screen until the owner moves it: {pill:?}");
        assert_eq!((frames[0].1 - pill.y, pill.y + pill.h - (frames[0].1 + frames[0].3)), (STATUS_INSET, STATUS_INSET));
        // a theme with another spacing scale moves the rhythm
        let mut theme = h.theme.clone();
        theme["space"] = json!([4, 6, 12, 16, 24, 24, 32, 40]);
        h.set_theme(theme);
        let again = h.status_frames();
        assert_eq!((again[1].0 - (again[0].0 + again[0].2), again[4].0 - (again[3].0 + again[3].2)), (6.0, 24.0));
    }

    #[test]
    fn switched_off_the_cameras_lose_their_button_and_their_card_and_get_them_back_when_switched_on() {
        let theme = lumatv_tokens::Tokens::shipped().json().clone();
        let shipped: Value = serde_json::from_str(include_str!("../../../../payload/theme/layout.json")).unwrap();
        let layout = json!({"grid": {"rows": 16}, "widgets": [{"id": "cams", "type": "cameras", "col": 0, "row": 0, "span": 6, "rows": 4},
                                         {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]});
        let mut h = Home::new(theme, shipped, layout);
        h.start(1000);
        let kinds = |h: &Home| h.status_items().iter().map(|it| kind_of(it).to_owned()).collect::<Vec<_>>();
        let cards = |h: &Home| h.cards().into_iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!((kinds(&h)[1].as_str(), cards(&h)), ("cameras", vec!["cams".to_owned()]), "no word from the helper is not off");
        let rev = h.chrome_rev();
        h.update(&Input::Helper { topic: "cameras".into(), data: json!({"state": "off", "cameras": []}) }, 1100);
        assert_eq!(kinds(&h), ["search", "notifications", "settings", "profile"]);
        assert!(cards(&h).is_empty());
        let card = h.widget_id("cams").unwrap();
        assert!(h.focus.frame(card).is_none(), "nothing to reach");
        assert_ne!(h.chrome_rev(), rev, "the cards are made again");
        h.update(&Input::Helper { topic: "cameras".into(), data: json!({"state": "unlinked", "cameras": []}) }, 1200);
        assert_eq!((kinds(&h)[1].as_str(), cards(&h)), ("cameras", vec!["cams".to_owned()]));
        assert!(h.focus.frame(card).is_some(), "the card is reached by the arrows again");
    }

    #[test]
    fn every_edge_of_the_bar_is_on_a_whole_pixel_at_every_text_size_and_name_width() {
        for scale in [1.0, 1.25, 1.5, 1.75, 2.0] {
            for name_width in [38.2, 47.63, 90.9] {
                let mut h = home();
                h.set_text_scale(scale);
                named(&mut h);
                h.set_name_width("Nitin".into(), name_width);
                let pill = h.status_pill();
                let whole = |v: f32| (v - v.round()).abs() < 1e-4;
                assert!([pill.x, pill.y, pill.w, pill.h].into_iter().all(whole), "pill at {scale} / {name_width}: {pill:?}");
                for f in h.status_frames() {
                    assert!([f.0, f.1, f.2, f.3].into_iter().all(whole), "button at {scale} / {name_width}: {f:?}");
                    assert_eq!(f.3 % 2.0, 0.0, "an even height: a glyph centred on it and a lift grown about it keep whole edges ({f:?})");
                }
                let chip = h.chip();
                assert!([chip.inset, chip.avatar, chip.name_x, chip.width].into_iter().all(whole), "chip at {scale} / {name_width}: {chip:?}");
                for f in [h.bell_frame(), h.camera_frame()] {
                    assert!(f.into_iter().all(whole), "{f:?}");
                }
            }
        }
    }

    #[test]
    fn the_camera_button_sits_beside_the_bell_and_asks_for_the_camera_list() {
        let mut h = home();
        let (bell, camera) = (h.bell_frame(), h.camera_frame());
        assert_eq!(camera[0] + camera[2] + h.status_gaps.0, bell[0], "next to the bell, a gap of the group's");
        while h.focus.current() != Some(Id(STATUS_ID + h.status_index("cameras").unwrap() as u32)) {
            press(&mut h, "right", 2000);
        }
        press(&mut h, "ok", 2100);
        assert_eq!(h.take_requests(), vec![Request::CameraList]);
        assert!(h.status_frames().len() == h.status_items().len(), "a frame for every button");
    }

    /// The owner placed the status bar (`layout.overlays.statusbar` of the theme).
    fn placed(h: &mut Home, anchor: &str, dx: f64) {
        let mut theme = h.theme.clone();
        theme["layout"] = json!({"overlays": {"statusbar": {"anchor": anchor, "dx": dx, "dy": 0}}});
        h.set_theme(theme);
    }

    const PLACES: [(&str, f64); 5] = [("top-left", 0.0), ("top", 0.0), ("top-right", 0.0), ("top", 0.2), ("top", -0.3)];

    #[test]
    fn the_status_bar_is_centred_by_default_follows_its_placement_and_keeps_its_buttons_and_their_order() {
        let mut h = home();
        named(&mut h);
        h.set_name_width("Nitin".into(), 47.63);
        let centre = |h: &Home| h.status_pill().x + h.status_pill().w / 2.0;
        assert!((centre(&h) - 960.0).abs() <= 0.5, "the middle of the screen until the owner moves it");
        let order: Vec<&str> = ["search", "cameras", "notifications", "settings", "profile"].to_vec();
        for (anchor, dx) in PLACES {
            placed(&mut h, anchor, dx);
            let pill = h.status_pill();
            assert!(pill.x >= 64.0 && pill.x + pill.w <= 1856.0 && pill.y == 54.0, "{anchor} {dx}: on the screen, in the safe area: {pill:?}");
            let frames = h.status_frames();
            assert!(frames.iter().all(|f| f.0 >= pill.x && f.0 + f.2 <= pill.x + pill.w && f.1 >= pill.y && f.1 + f.3 <= pill.y + pill.h), "{anchor} {dx}: {frames:?} in {pill:?}");
            assert!(frames.windows(2).all(|w| w[0].0 + w[0].2 < w[1].0), "left to right, no overlap");
            assert_eq!(order.iter().map(|k| h.status_index(k).unwrap()).collect::<Vec<_>>(), [0, 1, 2, 3, 4], "the D-pad order does not depend on where the pill is");
            // the focus engine has each button where it is drawn
            for (i, f) in frames.iter().enumerate() {
                let got = h.focus.frame(Id(STATUS_ID + i as u32)).map(|g| (g.x, g.w));
                assert_eq!(got, Some((f.0, f.2)), "{anchor} {dx}: button {i} is registered where it is drawn");
            }
        }
        placed(&mut h, "top-left", 0.0);
        assert_eq!(h.status_pill().x, 64.0);
        placed(&mut h, "top-right", 0.0);
        assert_eq!(h.status_pill().x + h.status_pill().w, 1856.0);
    }

    #[test]
    fn every_popups_way_in_is_its_buttons_laid_out_frame_wherever_the_pill_sits_and_at_every_text_size() {
        for scale in [1.0, 2.0] {
            let mut h = home();
            h.set_text_scale(scale);
            named(&mut h);
            h.set_name_width("Nitin".into(), 47.63);
            for (anchor, dx) in PLACES {
                placed(&mut h, anchor, dx);
                let frames = h.status_frames();
                for (kind, got) in [("cameras", h.camera_frame()), ("notifications", h.bell_frame()), ("settings", h.gear_frame()), ("profile", h.chip_frame())] {
                    let f = frames[h.status_index(kind).expect(kind)];
                    assert_eq!(got, [f.0, f.1, f.2, f.3], "{kind} at {anchor} {dx}, text {scale}: the button as the bar lays it out now");
                }
            }
        }
    }

    /// The owner placed the status bar and chose whether its glass spreads across the row (`full`).
    fn placed_as(h: &mut Home, anchor: &str, dx: f64, full: bool) {
        let mut theme = h.theme.clone();
        theme["layout"] = json!({"overlays": {"statusbar": {"anchor": anchor, "dx": dx, "dy": 0, "full": full}}});
        h.set_theme(theme);
    }

    #[test]
    fn full_width_spreads_the_glass_edge_to_edge_of_the_cards_grid_while_the_grouped_buttons_stay_where_the_placement_puts_them() {
        for scale in [1.0, 2.0] {
            for (anchor, dx) in PLACES {
                let tag = format!("{anchor} {dx}, text {scale}");
                let mut h = home();
                h.set_text_scale(scale);
                named(&mut h);
                h.set_name_width("Nitin".into(), 47.63);
                let (grid, dock) = (grid::area(&h.drawn_layout()), h.dock_rect().expect("the dock"));
                assert_eq!((grid.x, grid.w), (dock.x, dock.w), "{tag}: the dock spans the cards' grid");
                placed_as(&mut h, anchor, dx, false);
                let (hugging, frames) = (h.status_pill(), h.status_frames());
                assert_eq!(h.status_group(), hugging, "{tag}: without Full width the glass is the box of the buttons");
                // read live: the same theme with one more key
                placed_as(&mut h, anchor, dx, true);
                let (pill, group, spread) = (h.status_pill(), h.status_group(), h.status_frames());
                assert_eq!((pill.x, pill.x + pill.w), (grid.x as f32, (grid.x + grid.w) as f32), "{tag}: edge to edge of the cards' grid");
                assert_eq!((pill.y, pill.h), (hugging.y, hugging.h), "{tag}: in the same row, as tall");
                assert_eq!((group, &spread), (hugging, &frames), "{tag}: the buttons stay grouped where the placement puts them");
                assert!(group.x >= pill.x && group.x + group.w <= pill.x + pill.w, "{tag}: the group is inside the glass: {group:?} in {pill:?}");
                let (within, between) = h.status_gaps;
                for i in 0..4 {
                    let gap = spread[i + 1].0 - (spread[i].0 + spread[i].2);
                    assert_eq!(gap, if i == 3 { between } else { within }, "{tag}: the gaps are the theme's spacing ({i})");
                }
                let whole = |v: f32| (v - v.round()).abs() < 1e-4;
                assert!([pill.x, pill.y, pill.w, pill.h, group.x, group.w].into_iter().all(whole), "{tag}: whole pixels: {pill:?} {group:?}");
                assert!(spread.iter().all(|f| [f.0, f.1, f.2, f.3].into_iter().all(whole)), "{tag}: {spread:?}");
                // the focus engine has each button where it is drawn, and every popup grows from its button's laid-out frame
                for (i, f) in spread.iter().enumerate() {
                    assert_eq!(h.focus.frame(Id(STATUS_ID + i as u32)).map(|g| (g.x, g.w)), Some((f.0, f.2)), "{tag}: button {i} is registered where it is drawn");
                }
                for (kind, got) in [("cameras", h.camera_frame()), ("notifications", h.bell_frame()), ("settings", h.gear_frame()), ("profile", h.chip_frame())] {
                    let f = spread[h.status_index(kind).expect(kind)];
                    assert_eq!(got, [f.0, f.1, f.2, f.3], "{tag}: {kind}'s way in is the button as the bar lays it out now");
                }
                // the D-pad walks the buttons in the order it always did
                assert_eq!(h.focus.current(), Some(Id(STATUS_ID)), "{tag}: the focus starts on Search");
                for i in 1..5u32 {
                    press(&mut h, "right", 2000 + u64::from(i) * 100);
                    assert_eq!(h.focus.current(), Some(Id(STATUS_ID + i)), "{tag}: Right {i}");
                }
                for i in (0..4u32).rev() {
                    press(&mut h, "left", 3000 + u64::from(i) * 100);
                    assert_eq!(h.focus.current(), Some(Id(STATUS_ID + i)), "{tag}: Left to {i}");
                }
            }
        }
    }

    #[test]
    fn the_glass_follows_the_grids_edges_when_the_layouts_safe_margin_is_not_the_shipped_one() {
        let mut h = home();
        named(&mut h);
        h.set_name_width("Nitin".into(), 47.63);
        h.apply_layout(json!({"safe": {"x": 100}}));
        placed_as(&mut h, "top", 0.0, true);
        let (grid, pill) = (grid::area(&h.drawn_layout()), h.status_pill());
        assert_eq!((grid.x, grid.w), (100, 1720));
        assert_eq!((pill.x, pill.x + pill.w), (100.0, 1820.0), "the glass is as wide as the grid is now");
    }

    #[test]
    fn the_chip_keeps_the_last_person_the_helper_named_while_it_does_not_answer() {
        let mut h = home();
        let topic = json!({"ready": true, "active": "nitin", "owner": "nitin", "canAdmin": true, "switchedAt": 1,
            "profiles": [{"id": "nitin", "name": "Nitin", "initial": "N", "color": "#A79BF5", "active": true, "locked": false}]});
        let out = h.update(&Input::Helper { topic: "profiles".into(), data: topic }, 1500);
        let written = out.iter().find_map(|e| if let Effect::Write { path, text } = e { (path == "last-profile.json").then(|| text.clone()) } else { None });
        let written = written.expect("the person is kept in a file");
        // the helper stops: its topic is not ready, and the letter and name stay
        h.update(&Input::Helper { topic: "profiles".into(), data: json!({"ready": false}) }, 1600);
        assert_eq!(h.profile_name(), "Nitin");
        assert_eq!(h.profile().and_then(|p| p.get("initial")).and_then(Value::as_str), Some("N"));
        // a fresh start with the helper down reads the file
        let mut cold = home();
        let out = cold.start(1000);
        assert!(out.iter().any(|e| matches!(e, Effect::Read { tag, path } if *tag == KEPT_PROFILE_TAG && path == "last-profile.json")));
        assert_eq!(cold.profile_name(), "");
        cold.update(&Input::Fetched { tag: KEPT_PROFILE_TAG, status: 200, data: serde_json::from_str(&written).unwrap() }, 1010);
        assert_eq!(cold.profile_name(), "Nitin");
    }

    #[test]
    fn a_profile_switch_closes_what_was_open_and_reads_the_files_again() {
        let mut h = home();
        let topic = |active: &str, at: u64| json!({"ready": true, "active": active, "owner": "nitin", "canAdmin": true, "switchedAt": at,
            "profiles": [{"id": "nitin", "name": "Nitin", "initial": "N", "color": "#A79BF5", "active": active == "nitin", "locked": false},
                         {"id": "isabel", "name": "Isabel", "initial": "I", "color": "#EB6F92", "active": active == "isabel", "locked": false}]});
        h.update(&Input::Helper { topic: "profiles".into(), data: topic("nitin", 1) }, 1500);
        assert_eq!(h.profile_name(), "Nitin");
        press(&mut h, "1", 2000);
        assert_eq!(h.layer, Layer::Apps);
        h.update(&Input::Helper { topic: "profiles".into(), data: topic("nitin", 1) }, 2100);
        assert_eq!(h.layer, Layer::Apps, "the same person again changes nothing");
        let out = h.update(&Input::Helper { topic: "profiles".into(), data: topic("isabel", 2) }, 2200);
        assert_eq!(h.layer, Layer::None);
        assert_eq!(h.profile_name(), "Isabel");
        let out2 = h.update(&Input::Tick, 2201);
        assert!(out.iter().chain(out2.iter()).any(|e| matches!(e, Effect::Read { tag, .. } if *tag == THEME_TAG)), "the working copy is read at once");
    }

    #[test]
    fn words_heard_or_typed_elsewhere_land_in_the_search_field() {
        let mut h = home();
        h.update(&Input::Helper { topic: "voice".into(), data: json!({"ready": true, "state": "heard", "text": "net"}) }, 2000);
        assert_eq!(h.layer, Layer::Search);
        assert_eq!(h.search.text(), "net");
        assert_eq!(h.search.answers()[0].id, "netflix");
        h.update(&Input::Topic { topic: "keyboard.text".into(), data: json!({"text": "you"}) }, 2100);
        assert_eq!(h.search.text(), "you");
        let out = h.update(&Input::Topic { topic: "keyboard.text".into(), data: json!({"text": "you\n"}) }, 2200);
        assert_eq!(h.layer, Layer::None, "Done closes the sheet");
        assert!(!out.iter().any(|e| matches!(e, Effect::Luna { .. })), "Done on a search opens nothing by itself");
    }

    #[test]
    fn search_owns_digit_and_ok_repeats_and_pointer_clicks_use_the_shared_key_geometry() {
        let mut h = home();
        h.set_text_scale(2.0);
        press(&mut h, "search", 2000);
        press(&mut h, "7", 2100);
        assert_eq!(h.search.text(), "7", "digits type instead of opening Quick Access");
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 2200);
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: true }, 2600);
        h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 2700);
        assert_eq!(h.search.text(), "7g", "a held character key types once, without Home's long-press action");
        let key = h.search.key_frames()[15];
        let (x, y) = (key.x + key.w / 2.0, key.y + key.h / 2.0);
        for (at, buttons) in [(2800, 0), (2810, 1), (2820, 0)] {
            h.update(&Input::Pointer { x, y, buttons, visible: true }, at);
        }
        assert_eq!(h.search.text(), "7ge", "the pointer activates the key it clicked, not the previous keyboard focus");
        press(&mut h, "back", 2900);
        assert_eq!(h.layer, Layer::None);
    }

    #[test]
    fn a_late_settings_page_registers_the_new_search_answer_before_a_pointer_click() {
        let mut h = home();
        press(&mut h, "search", 2000);
        h.update(&Input::Topic { topic: "keyboard.text".into(), data: json!({"text": "net"}) }, 2100);
        assert_eq!(h.search.answers().len(), 2);
        h.set_search_pages(vec![json!({"id": "network", "title": "Network", "sub": "Settings"})]);
        assert_eq!(h.search.answers()[2].kind, "web");
        let f = h.search.answer_frame(2);
        let (x, y) = (f.x + f.w / 2.0, f.y + f.h / 2.0);
        let mut out = Vec::new();
        for (at, buttons) in [(2200, 0), (2210, 1), (2220, 0)] {
            out.extend(h.update(&Input::Pointer { x, y, buttons, visible: true }, at));
        }
        assert_eq!(h.layer, Layer::None);
        assert!(out.iter().any(|e| matches!(e, Effect::Luna { payload, .. } if payload["id"] == "com.webos.app.browser" && payload["params"]["target"].as_str().is_some_and(|s| s.contains("net")))));
    }


    #[test]
    fn the_picker_shows_on_wake_when_the_owner_asked_for_it() {
        let mut h = home();
        let mut topic = json!({"ready": true, "active": "nitin", "owner": "nitin", "canAdmin": true, "pickerOnWake": true, "switchedAt": 1,
            "profiles": [{"id": "nitin", "name": "Nitin", "initial": "N", "color": "#A79BF5", "active": true, "locked": false}]});
        h.update(&Input::Helper { topic: "profiles".into(), data: topic.clone() }, 1500);
        h.update(&Input::Topic { topic: "screen".into(), data: json!({"on": false}) }, 1600);
        h.update(&Input::Topic { topic: "screen".into(), data: json!({"on": true}) }, 1700);
        assert_eq!(h.layer, Layer::Picker);
        press(&mut h, "back", 1800);
        assert_eq!(h.layer, Layer::None, "Back keeps the current person");
        topic["pickerOnWake"] = json!(false);
        h.update(&Input::Helper { topic: "profiles".into(), data: topic }, 1900);
        h.update(&Input::Topic { topic: "screen".into(), data: json!({"on": false}) }, 2000);
        h.update(&Input::Topic { topic: "screen".into(), data: json!({"on": true}) }, 2100);
        assert_eq!(h.layer, Layer::None);
    }

    #[test]
    fn a_hover_is_a_cue_and_a_click_selects() {
        let mut h = home();
        let r = h.dock_rect().unwrap();
        let (sx, sy) = h.dock.slot(1);
        let (x, y) = (r.x as f32 + sx as f32 + 10.0, r.y as f32 + sy as f32 + 10.0);
        h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, 2000);
        assert_eq!(h.focus.hovered(), Some(Id(DOCK_ID + 1)));
        assert_eq!(h.state()["focusLayer"], "statusbar", "hover moved no focus");
        let out = h.update(&Input::Pointer { x, y, buttons: 1, visible: true }, 2100);
        assert!(!out.iter().any(|e| matches!(e, Effect::Luna { .. })), "the press selects; nothing opens yet");
        let out = h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, 2200);
        assert!(out.iter().any(|e| matches!(e, Effect::Luna { payload, .. } if payload["id"] == "netflix")), "a click opens what is under it when the button comes up over it");
        h.update(&Input::Key { name: "left".into(), down: true, repeat: false }, 2300);
        assert_eq!(h.focus.hovered(), None, "the next key hides the cue");
    }

    fn launches(out: &[Effect]) -> usize {
        out.iter().filter(|e| matches!(e, Effect::Luna { uri, .. } if uri.ends_with("/launch"))).count()
    }

    /// The pointer over the first dock tile, the tile's centre.
    fn over_tile(h: &Home) -> (f32, f32) {
        let r = h.dock_rect().unwrap();
        let (sx, sy) = h.dock.slot(1);
        (r.x as f32 + sx as f32 + 60.0, r.y as f32 + sy as f32 + 60.0)
    }

    #[test]
    fn a_pointer_that_goes_to_sleep_opens_nothing() {
        let mut h = home();
        let (x, y) = over_tile(&h);
        h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, 2000);
        h.update(&Input::Key { name: "right".into(), down: true, repeat: false }, 2100);
        let mut all = Vec::new();
        // the remote lies still: the pointer is hidden, with the button up and then, as a stray sample, with it down
        for (buttons, visible, at) in [(0, false, 2200), (1, false, 2210), (0, false, 2220), (0, true, 2230)] {
            all.extend(h.update(&Input::Pointer { x, y, buttons, visible }, at));
        }
        assert_eq!(launches(&all), 0, "{all:?}");
        // a release with no press seen is nothing, with the pointer shown
        all.extend(h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, 2300));
        assert_eq!(launches(&all), 0);
    }

    #[test]
    fn a_button_already_down_when_the_pointer_shows_is_not_a_press_and_its_release_is_not_a_click() {
        let mut h = home();
        let (x, y) = over_tile(&h);
        let mut all = Vec::new();
        for (buttons, at) in [(1, 2000), (1, 2020), (0, 2040)] {
            all.extend(h.update(&Input::Pointer { x, y, buttons, visible: true }, at));
        }
        assert_eq!(launches(&all), 0, "{all:?}");
        // the same pointer, now armed, clicks as it should
        for (buttons, at) in [(1, 2100), (0, 2140)] {
            all.extend(h.update(&Input::Pointer { x, y, buttons, visible: true }, at));
        }
        assert_eq!(launches(&all), 1);
    }

    #[test]
    fn a_pointer_that_hides_or_leaves_the_tile_between_the_press_and_the_release_clicks_nothing() {
        let mut h = home();
        let (x, y) = over_tile(&h);
        let mut all = Vec::new();
        h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, 2000);
        h.update(&Input::Pointer { x, y, buttons: 1, visible: true }, 2020);
        all.extend(h.update(&Input::Pointer { x, y, buttons: 0, visible: false }, 2040));
        all.extend(h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, 2060));
        assert_eq!(launches(&all), 0, "hiding between press and release cancels it");
        h.update(&Input::Pointer { x, y, buttons: 1, visible: true }, 2100);
        all.extend(h.update(&Input::Pointer { x: 5.0, y: 5.0, buttons: 0, visible: true }, 2140));
        assert_eq!(launches(&all), 0, "released somewhere else");
    }

    fn notes_of(h: &mut Home) -> Vec<Note> {
        h.take_notes()
    }

    #[test]
    fn a_button_held_on_a_tile_opens_its_options_and_neither_its_release_nor_a_key_release_after_it_opens_or_picks_anything() {
        let mut h = home();
        let (x, y) = over_tile(&h);
        let mut all = Vec::new();
        h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, 3000);
        all.extend(h.update(&Input::Pointer { x, y, buttons: 1, visible: true }, 3020));
        assert_eq!(h.layer, Layer::None);
        all.extend(h.update(&Input::Tick, 3020 + h.hold_ms + 10));
        assert_eq!(h.layer, Layer::Menu, "the hold opened the options");
        all.extend(h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, 3020 + h.hold_ms + 60));
        assert_eq!(h.layer, Layer::Menu, "the release did not pick a row");
        assert_eq!(h.menu_index, 0);
        assert_eq!(launches(&all), 0, "{all:?}");
        // Back leaves it; a stray OK release after that is nothing
        press(&mut h, "back", 4000);
        all.extend(h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 4100));
        assert_eq!((h.layer, launches(&all)), (Layer::None, 0));
    }

    #[test]
    fn a_held_ok_that_opens_the_options_launches_nothing_whether_the_hold_is_found_at_a_tick_or_at_the_release() {
        let mut h = home();
        press(&mut h, "2", 1500);
        h.close_all();
        let mut all = Vec::new();
        // no tick between the press and its release: the release finds the hold and is the hold's
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 3000);
        all.extend(h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 3000 + h.hold_ms + 20));
        assert_eq!(h.layer, Layer::Menu);
        assert_eq!((h.menu_index, launches(&all)), (0, 0), "{all:?}");
        // with the remote's repeats between, the same
        press(&mut h, "back", 3500);
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 4000);
        h.update(&Input::Tick, 4000 + h.hold_ms + 5);
        all.extend(h.update(&Input::Key { name: "ok".into(), down: true, repeat: true }, 4800));
        all.extend(h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 4900));
        assert_eq!((h.layer, h.menu_index, launches(&all)), (Layer::Menu, 0, 0), "{all:?}");
    }

    #[test]
    fn an_app_that_opens_tells_the_launch_screen_its_name_its_icon_and_where_its_tile_is_grown_by_the_lift_when_the_tile_had_the_focus() {
        let mut h = home();
        to_the_dock_under_the_home_card(&mut h, 2000);
        assert_eq!(h.focus.current(), Some(Id(DOCK_ID + 1)));
        press(&mut h, "ok", 2400);
        let notes = notes_of(&mut h);
        let Some(Note::Launch { id, title, icon, tile: Some(tile) }) = notes.first() else { panic!("a launch: {notes:?}") };
        assert_eq!((id.as_str(), title.as_str(), icon.as_str()), ("netflix", "Netflix", "/netflix.png"));
        let card = h.dock_rect().unwrap();
        let (sx, sy) = h.dock.slot(1);
        let (cx, cy) = (card.x as f32 + sx as f32 + h.dock.tile as f32 / 2.0, card.y as f32 + sy as f32 + h.dock.tile as f32 / 2.0);
        let k = lumatv_tokens::Tokens::shipped().focus.tile_scale;
        assert!((tile.w - h.dock.tile as f32 * k).abs() < 0.01 && (tile.x + tile.w / 2.0 - cx).abs() < 0.01 && (tile.y + tile.h / 2.0 - cy).abs() < 0.01, "the lifted tile about its centre: {tile:?}");
        assert!((tile.radius - lumatv_tokens::Tokens::shipped().radius(lumatv_tokens::Radius::Tile) * k).abs() < 0.01);
        // the library's launch of the same app is not from a lifted tile, and an app that is not on the dock has no tile at all
        h.open_apps(&mut Vec::new());
        let mut out = Vec::new();
        h.launch("netflix", &json!({}), "Netflix", &mut out);
        let notes = notes_of(&mut h);
        assert!(matches!(&notes[0], Note::Launch { tile: Some(t), .. } if (t.w - h.dock.tile as f32).abs() < 0.01), "{notes:?}");
        h.launch("not.on.the.dock", &json!({}), "", &mut out);
        assert!(matches!(&notes_of(&mut h)[0], Note::Launch { title, tile: None, icon, .. } if title == "not.on.the.dock" && icon.is_empty()));
    }

    #[test]
    fn the_web_search_opens_the_browser_with_no_launch_screen() {
        let mut h = home();
        let mut out = Vec::new();
        h.start_app("com.webos.app.browser", &json!({"target": "https://x"}), "Browser", &mut out);
        assert!(notes_of(&mut h).is_empty());
        assert!(out.iter().any(|e| matches!(e, Effect::Luna { payload, .. } if payload["id"] == "com.webos.app.browser")));
    }

    fn with_quick_access(h: &mut Home, slots: Value) {
        h.theme["remote"] = json!({"quickAccess": {"on": true, "slots": slots}});
    }

    #[test]
    fn with_quick_access_on_a_digit_says_what_it_opens_and_the_owners_own_choice_comes_before_the_dock() {
        let mut h = home();
        press(&mut h, "3", 1500);
        assert!(!notes_of(&mut h).iter().any(|n| matches!(n, Note::Toast { .. })), "off: the number keys are what they were and say nothing");
        with_quick_access(&mut h, json!({}));
        press(&mut h, "3", 2000);
        let notes = notes_of(&mut h);
        assert!(matches!(&notes[..], [Note::Launch { id, .. }, Note::Toast { digit, title, line }] if id == "youtube" && digit == "3" && title == "YouTube" && line == "Opening."), "{notes:?}");
        with_quick_access(&mut h, json!({"3": "plex"}));
        let out = press(&mut h, "3", 2200);
        assert!(out.iter().any(|e| matches!(e, Effect::Luna { payload, .. } if payload["id"] == "plex")), "the owner's choice: not the dock's third tile");
        assert!(matches!(&notes_of(&mut h)[..], [Note::Launch { id, .. }, Note::Toast { title, .. }] if id == "plex" && title == "Plex"));
        // the Apps tile opens the library, which is no launch to speak of
        press(&mut h, "1", 2400);
        assert_eq!(h.layer, Layer::Apps);
        assert!(notes_of(&mut h).is_empty());
    }

    #[test]
    fn a_digit_with_nothing_behind_it_asks_with_the_apps_no_digit_opens_and_the_answer_is_kept_and_opens_the_app() {
        let mut h = home();
        h.theme["apps"] = json!({"dockHide": ["extra.a", "extra.b", "extra.c", "extra.d", "extra.e"], "hidden": ["extra.e"]});
        let mut lps = vec![json!({"id": APPS_TILE, "title": "Apps", "unmovable": true, "icon": "/a.png"})];
        for (id, t) in [("netflix", "Netflix"), ("youtube", "YouTube"), ("plex", "Plex"), ("extra.a", "A"), ("extra.b", "B"), ("extra.c", "C"), ("extra.d", "D"), ("extra.e", "E")] {
            lps.push(json!({"id": id, "title": t, "icon": format!("/{id}.png")}));
        }
        h.update(&Input::Luna { tag: LP_TAG, payload: json!({"launchPoints": lps}) }, 1100);
        with_quick_access(&mut h, json!({"7": "extra.a"}));
        press(&mut h, "9", 2000);
        let notes = notes_of(&mut h);
        let [Note::Ask { digit, picks }] = &notes[..] else { panic!("a question: {notes:?}") };
        assert_eq!(digit, "9");
        assert_eq!(picks.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["extra.b", "extra.c", "extra.d"], "not the one a digit opens already, not the hidden one, not the dock's");
        assert!(h.layer == Layer::None && h.dock.focused().is_some_and(|p| p.id != "extra.b"), "asking opened nothing");
        let mut out = Vec::new();
        h.extras_intent(Intent::Assign { digit: "9".into(), app: "extra.b".into() }, &mut out);
        assert_eq!(h.theme.pointer("/remote/quickAccess/slots/9"), Some(&json!("extra.b")));
        assert_eq!(h.theme.pointer("/remote/quickAccess/slots/7"), Some(&json!("extra.a")), "the others stay");
        assert!(out.iter().any(|e| matches!(e, Effect::Theme { patch, .. } if patch["remote"]["quickAccess"]["slots"]["9"] == "extra.b" && patch["remote"]["quickAccess"]["on"] == true)), "kept for the next start");
        assert!(out.iter().any(|e| matches!(e, Effect::Luna { payload, .. } if payload["id"] == "extra.b")), "and the app opens");
        assert!(matches!(&notes_of(&mut h)[..], [Note::Launch { .. }, Note::Toast { line, .. }] if line == "Assigned. Opening."));
        h.extras_intent(Intent::More("9".into()), &mut out);
        assert_eq!(h.take_requests(), vec![Request::SettingsPage("remote".into())]);
    }

    #[test]
    fn the_remote_key_opens_the_on_screen_remote_over_a_quiet_home_and_a_story_opens_the_frame_out_of_its_card() {
        let mut h = home();
        press(&mut h, "screenremote", 2000);
        assert_eq!(notes_of(&mut h), vec![Note::Remote]);
        h.open_apps(&mut Vec::new());
        press(&mut h, "screenremote", 2200);
        assert!(notes_of(&mut h).is_empty(), "not over the library");
        h.close_all();
        h.news = json!({"doc": {"items": [{"link": "https://news.example/a", "title": "A story", "src": "BBC", "pubMs": 1_000u64}]}});
        let card = h.cards().into_iter().find(|e| e.kind == "news").expect("the shipped layout has a news card");
        let mut out = Vec::new();
        h.now = 1_000 + 3 * 3_600_000;
        assert!(h.inner_key(&card.id, "ok", &mut out));
        let notes = notes_of(&mut h);
        let [Note::Frame { page, from, to }] = &notes[..] else { panic!("a frame: {notes:?}") };
        assert_eq!((page.url.as_str(), page.title.as_str(), page.source.as_str(), page.ago.as_str()), ("https://news.example/a", "A story", "BBC", "3 h ago"));
        assert_eq!(page.subtitle, "BBC \u{b7} 3 h ago");
        let r = h.rect(&card);
        assert_eq!(*from, [r.x as f32, r.y as f32, r.w as f32, r.h as f32]);
        assert_eq!((to[0], to[1], to[2], to[3]), (200.0, 150.0, 1520.0, 860.0), "centred under the status bar");
        assert!(!out.iter().any(|e| matches!(e, Effect::Luna { .. })), "the browser is not opened: the frame is");
    }

    #[test]
    fn the_hole_takes_the_picture_away_only_once_the_clip_plays_and_has_settled_and_the_picture_is_back_when_home_stops_wanting_it() {
        use crate::home::wallvideo::{MANIFEST_TAG, STATUS_TAG};
        let mut h = home();
        h.theme["wallpaperMotion"]["source"] = json!("video");
        let manifest: Value = serde_json::from_str(include_str!("../../../../payload/vplane/clips/wall.json")).unwrap();
        h.update(&Input::Topic { topic: "wall".into(), data: json!({"hole": true}) }, 1100);
        assert!(!h.wall_hole, "the compositor's hole alone is not the clip: Home's own picture stays until the player plays");
        let out = h.update(&Input::Tick, 1200);
        assert!(out.iter().any(|e| matches!(e, Effect::Read { tag: MANIFEST_TAG, .. })), "the clips' manifest is read");
        // Home is in front with the screen on and has the manifest: it wants the clip, and asks for the player's status; the player says idle, the start goes out
        let out = h.update(&Input::Fetched { tag: MANIFEST_TAG, status: 200, data: manifest }, 1210);
        assert!(out.iter().any(|e| matches!(e, Effect::Read { tag: STATUS_TAG, .. })), "{out:?}");
        let out = h.update(&Input::Fetched { tag: STATUS_TAG, status: 200, data: json!({"ts": 1215, "seq": 1, "role": "saver", "state": "idle"}) }, 1230);
        let start = out.iter().find_map(|e| match e {
            Effect::Write { path, text } if path == crate::home::wallvideo::REQ_FILE => serde_json::from_str::<Value>(text).ok(),
            _ => None,
        });
        let start = start.expect("a start request");
        assert_eq!((start["op"].as_str(), start["clip"].as_str()), (Some("start"), Some("wall-silk")));
        h.update(&Input::Tick, 1450);
        h.update(&Input::Fetched { tag: STATUS_TAG, status: 200, data: json!({"ts": 1440, "seq": start["seq"], "role": "wall", "state": "playing"}) }, 1460);
        assert!(!h.wall_hole, "playing, but the first picture has not settled");
        let out = h.update(&Input::Tick, 1470 + 250);
        assert!(h.wall_hole && out.contains(&Effect::Redraw), "settled with the hole under Home: Home's picture and scrim go");
        assert_eq!(h.state()["wallHole"], true);
        assert_eq!(h.state()["wall"]["state"], "playing");
        // the owner turns the switch off: the picture is back in the same update
        h.theme["wallpaperMotion"]["source"] = json!("gpu");
        let out = h.update(&Input::Tick, 2000);
        assert!(!h.wall_hole && out.contains(&Effect::Redraw));
    }

    #[test]
    fn a_search_answer_that_is_a_settings_page_asks_the_shell_for_that_page() {
        let mut h = home();
        press(&mut h, "search", 2000);
        assert!(h.take_pages_wanted(), "Search has opened and wants the pages the shell has");
        h.set_search_pages(vec![json!({"id": "network", "title": "Network", "icon": "wifi", "sub": "Settings \u{b7} Connected"}), json!({"id": "sound", "title": "Sound", "icon": "volume-2", "sub": "Settings"})]);
        h.update(&Input::Topic { topic: "keyboard.text".into(), data: json!({"text": "net"}) }, 2100);
        let answers = h.search.answers();
        assert_eq!(answers.iter().map(|a| (a.kind, a.id.as_str())).collect::<Vec<_>>(), vec![("app", "netflix"), ("page", "network"), ("web", "web")]);
        assert_eq!(answers[1].sub, "Settings \u{b7} Connected");
        for i in 0..3 { press(&mut h, "up", 2200 + i * 100); }
        assert!(h.search.answer_index().is_some());
        while h.search.answer_index() != Some(1) {
            let key = if h.search.answer_index().unwrap() > 1 { "left" } else { "right" };
            press(&mut h, key, 2600);
        }
        let out = press(&mut h, "ok", 2800);
        assert_eq!(h.take_requests(), vec![Request::SettingsPage("network".into())]);
        assert_eq!(h.layer, Layer::None, "the sheet is closed behind it");
        assert!(!out.iter().any(|e| matches!(e, Effect::Luna { .. })), "nothing is launched");
        // a page picked is not an app: the next Search starts from the pages the shell gave
        press(&mut h, "search", 3000);
        assert!(h.take_pages_wanted() && h.search.answers().is_empty());
    }

    fn gear(h: &mut Home) {
        let at = h.status_index("settings").expect("the gear");
        for i in 0..8 {
            if h.focus.current() == Some(Id(STATUS_ID + at as u32)) {
                break;
            }
            press(h, "right", 1500 + i * 100);
        }
        h.open_popover(at, &mut Vec::new());
    }

    #[test]
    fn wallpaper_popover_values_change_at_once_and_are_written_to_the_theme_when_it_closes() {
        let mut h = home();
        gear(&mut h);
        assert_eq!(h.layer, Layer::Popover);
        assert!(h.take_requests().is_empty());
        press(&mut h, "down", 2200);
        assert_eq!(h.state()["popover"]["row"], "amplitude");
        let out = press(&mut h, "right", 2300);
        assert_eq!(h.theme["wallpaperMotion"]["amplitude"], 5.0, "Home behind has it at once");
        assert!(out.contains(&Effect::Redraw));
        assert_eq!(h.state()["popover"]["values"][1], "5 px");
        press(&mut h, "right", 2400);
        press(&mut h, "up", 2500);
        press(&mut h, "ok", 2600);
        assert_eq!(h.theme["wallpaperMotion"]["enabled"], false, "OK on the toggle turns it");
        let out = press(&mut h, "back", 2700);
        assert_eq!(h.layer, Layer::None);
        assert_eq!(h.focus.current(), Some(Id(STATUS_ID + h.status_index("settings").unwrap() as u32)), "the gear has the focus again");
        let patch = out.iter().find_map(|e| match e {
            Effect::Theme { tag: POPOVER_TAG, patch } => Some(patch.clone()),
            _ => None,
        });
        assert_eq!(patch, Some(json!({"wallpaperMotion": {"amplitude": 6.0, "enabled": false}})), "what was changed, once, at the close");
        assert!(!press(&mut h, "down", 2900).iter().any(|e| matches!(e, Effect::Theme { tag: POPOVER_TAG, .. })));
    }

    #[test]
    fn tv_settings_opens_and_pointer_controls_and_closes_the_wallpaper_panel() {
        let mut h = home();
        gear(&mut h);
        for i in 0..12 {
            press(&mut h, "down", 2300 + i * 40);
        }
        let out = press(&mut h, "ok", 2900);
        assert!(out.iter().any(|e| matches!(e, Effect::Luna { payload, .. } if payload["id"] == "com.palm.app.settings")), "TV settings: LG's");
        assert!(notes_of(&mut h).is_empty(), "no launch screen for it");
        // the pointer: a click on a slider's row is a step up, on the toggle it turns, away from the panel it closes
        gear(&mut h);
        let rows = h.popover_lay.clone().unwrap().rows;
        let centre = |r: &ui::geometry::Frame| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let click = |h: &mut Home, (x, y): (f32, f32), at: u64| {
            h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, at);
            h.update(&Input::Pointer { x, y, buttons: 1, visible: true }, at + 20);
            h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, at + 40);
        };
        click(&mut h, centre(&rows[2].1), 3000);
        assert_eq!(h.theme["wallpaperMotion"]["speed"], 1.1, "Speed took a step up");
        click(&mut h, centre(&rows[0].1), 3100);
        assert_eq!(h.theme["wallpaperMotion"]["enabled"], false);
        assert_eq!(h.layer, Layer::Popover, "a row's click leaves it up");
        click(&mut h, (200.0, 900.0), 3200);
        assert_eq!(h.layer, Layer::None, "a click elsewhere puts it away");
    }
}
