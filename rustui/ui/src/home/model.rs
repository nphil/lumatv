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
use super::popover::{self, Done, Popover};
use super::search::{Search, SearchOut};
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
pub const TINT_TAG: u64 = 312;
pub const SWITCH_TAG: u64 = 320;
/// The answer of a control of a smart-home tile (`POST /v1/onhome/act`).
pub const ONHOME_TAG: u64 = 330;
/// The read of the token the helper keeps for the shell to unlock with (`onhome/shell.json`), asked only after the person pressed Unlock.
pub const UNLOCK_TAG: u64 = 331;
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
    /// The app switcher: Back from the top of Home (when the owner wants it) and a press of Home with nothing open on Home.
    Switcher,
    Settings,
    /// Settings at one of its pages (Quick Access's More, a Search answer that is a page).
    SettingsPage(String),
    Notifications,
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

pub struct Home {
    pub theme: Value,
    pub layout: Value,
    pub shipped_layout: Value,
    pub layout_text: String,
    pub theme_text: String,
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
    /// A full-screen modal (Settings, the display lab) is up over Home: what moves under it is not seen, so the weather's sprites rest.
    covered: bool,
    /// Something is over Home (a panel, or the modal above): the GPU wallpaper stands still while it is, so that its steps do not owe the whole screen.
    overlaid: bool,
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

impl Home {
    /// `theme` is the merged theme (the shipped under the owner's), `shipped_layout` the shipped layout and `user_layout` the owner's file.
    pub fn new(theme: Value, shipped_layout: Value, user_layout: Value) -> Self {
        let motion = theme.get("motion").cloned().unwrap_or(Value::Null);
        let rest_ms = motion.get("restMs").and_then(Value::as_u64).unwrap_or(150);
        let hold_ms = motion.get("holdMs").and_then(Value::as_u64).unwrap_or(700);
        let mut h = Self {
            layout: Value::Null,
            layout_text: serde_json::to_string(&user_layout).unwrap_or_default(),
            theme_text: String::new(),
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
            popover_lay: None,
            persist: None,
            active: false,
            wall_hole: false,
            wall_topic: false,
            wall: WallVideo::default(),
            light: false,
            saver_up: false,
            covered: false,
            overlaid: false,
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
        self.layout = grid::tidy_layout(&merged);
        for e in self.cards() {
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
        self.layout = layout;
        self.register();
        self.dirty = true;
    }

    pub fn set_theme(&mut self, theme: Value) {
        self.theme = theme;
        self.dock.refresh(&self.theme);
        self.rest_ms = self.theme.pointer("/motion/restMs").and_then(Value::as_u64).unwrap_or(150);
        self.hold_ms = self.theme.pointer("/motion/holdMs").and_then(Value::as_u64).unwrap_or(700);
        self.register();
        self.dirty = true;
    }

    /// The cards on Home (not the dock, not hidden), in the layout's order.
    pub fn cards(&self) -> Vec<Entry> {
        grid::entries(&self.layout).into_iter().filter(|e| !e.is_dock() && !e.hidden).collect()
    }

    pub fn dock_entry(&self) -> Option<Entry> {
        grid::entries(&self.layout).into_iter().find(Entry::is_dock)
    }

    /// The layout as it is drawn: the status bar at the text size's height.
    fn drawn_layout(&self) -> Value {
        grid::with_status_height(&self.layout, self.status_height())
    }

    pub fn rect(&self, e: &Entry) -> Rect {
        grid::rect_of(&self.drawn_layout(), e)
    }

    pub fn dock_rect(&self) -> Option<Rect> {
        self.dock_entry().map(|d| self.rect(&d))
    }

    /// The status bar's items, left then right, as `{type, icon}` objects.
    pub fn status_items(&self) -> Vec<Value> {
        let sb = self.layout.get("statusBar");
        let mut out: Vec<Value> = sb.and_then(|s| s.get("left")).and_then(Value::as_array).cloned().unwrap_or_default();
        out.extend(sb.and_then(|s| s.get("right")).and_then(Value::as_array).cloned().unwrap_or_default());
        out
    }

    pub fn left_count(&self) -> usize {
        self.layout.pointer("/statusBar/left").and_then(Value::as_array).map_or(0, Vec::len)
    }

    pub fn safe(&self) -> (f32, f32) {
        (self.layout.pointer("/safe/x").and_then(Value::as_f64).unwrap_or(64.0) as f32, self.layout.pointer("/safe/y").and_then(Value::as_f64).unwrap_or(54.0) as f32)
    }

    /// The bar's height grows with the text size (the layout's 60 at 100 %), as the chip's name does.
    pub fn status_height(&self) -> f32 {
        (self.layout.pointer("/statusBar/height").and_then(Value::as_f64).unwrap_or(60.0) as f32 * self.text_scale.max(1.0)).round()
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
            self.apps.scale = scale;
            self.register();
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

    /// The status bar's buttons sit in a pill at the safe corner: item `i` is `48 + 6` apart, left from 6, right ending 6 in (the same gap as at the left, so every button is concentric with the pill's cap).
    pub fn status_frames(&self) -> Vec<(f32, f32, f32, f32)> {
        let (sx, sy) = self.safe();
        let items = self.status_items();
        let left = self.left_count();
        let w = 1920.0 - 2.0 * sx;
        let h = self.status_height();
        let item_h = h - 12.0;
        let mut out = Vec::new();
        let mut x = sx + 6.0;
        for (i, it) in items.iter().enumerate() {
            let iw = self.status_item_width(it);
            if i < left {
                out.push((x, sy + 6.0, iw, item_h));
                x += iw + 6.0;
            }
        }
        let right: Vec<f32> = items.iter().skip(left).map(|it| self.status_item_width(it)).collect();
        let total: f32 = right.iter().sum::<f32>() + 6.0 * right.len().saturating_sub(1) as f32;
        let mut rx = sx + w - 6.0 - total;
        for iw in right {
            out.push((rx, sy + 6.0, iw, item_h));
            rx += iw + 6.0;
        }
        out
    }

    /// A button is a square of the pill's inner height; the profile chip adds its name.
    pub fn status_item_width(&self, item: &Value) -> f32 {
        let h = self.status_height() - 12.0;
        if item.get("type").and_then(Value::as_str) == Some("profile") { self.chip().width } else { h }
    }

    /// The profile chip's measures at the status bar's button height.
    pub fn chip(&self) -> Chip {
        let name = self.profile_name();
        let h = self.status_height() - 12.0;
        if name.is_empty() {
            return Chip::new(h, None);
        }
        let scale = self.text_scale.max(1.0);
        let w = match &self.name_width {
            Some((n, s, w)) if *n == name && (*s - scale).abs() < 0.001 => *w,
            _ => 9.0 * scale * name.chars().count() as f32,
        };
        Chip::new(h, Some(w))
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

    /// Where the bell is on the status bar (`[x, y, w, h]`), the notification drawer's way in and out. With no bell in the layout, or Home
    /// not drawn (an app in front, the drawer opened by a key), where it would be: the right end of the bar.
    pub fn bell_frame(&self) -> [f32; 4] {
        let at = self.status_items().iter().position(|it| it.get("type").and_then(Value::as_str) == Some("notifications"));
        if let Some(&(x, y, w, h)) = at.and_then(|i| self.status_frames().get(i).copied()).as_ref() {
            return [x, y, w, h];
        }
        let (sx, sy) = self.safe();
        let side = self.status_height() - 12.0;
        [1920.0 - sx - 6.0 - side, sy + 6.0, side, side]
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

    /// What is over Home: `covered` for a full-screen modal (Settings, the display lab), `overlaid` for that or a panel. A frame that Home's own
    /// motion makes under them is a frame of the whole stack for nothing: the GPU wallpaper steps ten times a second and each step owes the
    /// screen to every page of the ring (round 22's ring at 120 Hz: three frames of 157 layers, 13-16 ms of GPU each, under Settings), and the
    /// weather's sprites a frame every refresh. Returns whether it changed, so that Home is drawn again.
    pub fn set_overlay(&mut self, covered: bool, overlaid: bool) -> bool {
        let changed = covered != self.covered || overlaid != self.overlaid;
        self.covered = covered;
        self.overlaid = overlaid;
        changed
    }

    /// Whether the GPU wallpaper stands still: something is over Home.
    pub fn wallpaper_held(&self) -> bool {
        self.overlaid
    }

    fn camera_stills_visible(&self) -> bool {
        self.active && self.screen_on && self.layer == Layer::None && self.cameras.get("state").and_then(Value::as_str) == Some("ready")
    }

    fn tick(&mut self, out: &mut Vec<Effect>) {
        let now = self.now;
        if let Some(at) = self.ok_down_at
            && !self.ok_held
            && now >= at + self.hold_ms
        {
            self.ok_held = true;
            self.hold(out);
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
        if self.search.tick(now) {
            self.redraw(out);
        }
        if self.onhome_said.as_ref().is_some_and(|(_, until)| now >= *until) {
            self.onhome_said = None;
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
        if let Some(at) = self.ok_down_at.filter(|_| !self.ok_held) {
            take(Some(at + self.hold_ms));
        }
        if self.moved_at > 0 && self.now < self.moved_at + self.rest_ms {
            take(Some(self.moved_at + self.rest_ms));
        }
        if self.started && self.active {
            take(Some(self.next_poll));
        }
        take(self.editor.as_ref().and_then(Editor::due_at));
        take(self.picker.due_at());
        take(self.search.due_at());
        take(self.stills.due_at(self.camera_stills_visible()));
        take(self.wall.due_at());
        take(self.radar.due_at(self.radar_loops()));
        take(self.onhome_said.as_ref().map(|(_, until)| *until));
        if let Some(sheet) = &self.sheet {
            take(sheet.waiting());
            take(sheet.notice.as_ref().map(|(_, until)| *until));
        }
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
            "cameras" => self.cameras = data.clone(),
            "onhome" => {
                self.onhome = data.clone();
                // the thing whose sheet is open is gone from Home: the sheet goes with it
                if self.layer == Layer::OnHome && self.sheet_tile().is_none() {
                    self.close_layer();
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
                        let merged = merge_over(&lumatv_tokens::Tokens::shipped().json().clone(), data);
                        self.set_theme(merged);
                        self.redraw(out);
                    }
                }
            }
            LAYOUT_TAG => {
                self.poll_outstanding = self.poll_outstanding.saturating_sub(1);
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
            ONHOME_TAG if status != 200 => {
                let words = data.pointer("/error/message").and_then(Value::as_str).filter(|w| !w.is_empty()).unwrap_or("Home Assistant did not do that").to_owned();
                self.onhome_said = Some((words.clone(), self.now + 4000));
                if let Some(sheet) = self.sheet.as_mut() {
                    sheet.say(&words, self.now);
                }
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
        if let Some(d) = name.chars().next().filter(|c| c.is_ascii_digit() && name.len() == 1) {
            if down && !repeat {
                if self.layer == Layer::Picker {
                    self.picker_key(name, None, out);
                    self.redraw(out);
                } else if self.layer == Layer::Weather {
                    // the radar page's views are 1 Home, 2 Region, 3 Globe
                    self.weather_key(name, None, out);
                    self.redraw(out);
                } else {
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
                return;
            }
            // a release with no press of Home's before it is the end of a press that another kind took (it closed as the press came): no OK
            let Some(at) = self.ok_down_at else {
                return;
            };
            // a release that comes after the hold's time without a tick in between (a slow frame) is the hold's release
            if !self.ok_held && self.now >= at + self.hold_ms
            {
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
            Layer::None if self.inner.as_deref().is_some_and(|w| self.kind_of(w) == "onhome") => self.open_sheet(out),
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
        // the Menu key on a smart-home tile is its controls, as a held OK is
        if name == "menu" && self.inner.as_deref().is_some_and(|w| self.kind_of(w) == "onhome") {
            self.open_sheet(out);
            return;
        }
        if name == "menu" {
            if self.focus.current().is_some_and(|id| Self::dock_index_of(id).is_some()) {
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
        if let Some(wid) = self.widget_of(cur).map(str::to_owned) {
            match (name, dir) {
                ("ok", _) => {
                    if self.has_inner(&wid) {
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
                    "settings" => self.open_popover(i, out),
                    "notifications" => self.requests.push(Request::Notifications),
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
            Some("cameras" | "news") => true,
            Some("onhome") => !self.onhome_tiles().is_empty(),
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

    /// What the card says of the last control that was refused, while it is said.
    pub fn onhome_notice(&self) -> Option<&str> {
        self.onhome_said.as_ref().map(|(w, _)| w.as_str())
    }

    /// The tile the open sheet is for, as the topic has it now.
    pub fn sheet_tile(&self) -> Option<Value> {
        let id = &self.sheet.as_ref()?.id;
        self.onhome_tiles().into_iter().find(|t| t.get("id").and_then(Value::as_str) == Some(id.as_str()))
    }

    /// Tiles to a row on the card `wid` (what the widget draws, so the keys move as the eye does).
    fn onhome_cols(&self, wid: &str) -> usize {
        let Some(entry) = self.cards().into_iter().find(|e| e.id == wid) else { return 1 };
        super::onhome::cols(self.rect(&entry).w as f32 - 2.0 * super::widgets::PAD_SIDE)
    }

    fn send_act(&mut self, act: &super::onhome::Act, out: &mut Vec<Effect>) {
        out.push(Effect::Request { tag: ONHOME_TAG, method: "POST".into(), path: "/v1/onhome/act".into(), body: Some(json!({"id": act.id, "verb": act.verb, "value": act.value})) });
    }

    /// The controls of the tile in front on the card with the inner focus.
    fn open_sheet(&mut self, out: &mut Vec<Effect>) {
        let Some(tile) = self.onhome_tiles().get(self.inner_index.unwrap_or(0)).cloned() else { return };
        let Some(id) = tile.get("id").and_then(Value::as_str) else { return };
        self.sheet = Some(super::onhome::Sheet::new(id));
        self.open_layer(Layer::OnHome, SCOPE_ONHOME);
        self.redraw(out);
    }

    /// OK on a locked lock's tile: the sheet opens on "Unlock <name>?" with Cancel in front.
    fn open_unlock_question(&mut self, out: &mut Vec<Effect>) {
        let Some(tile) = self.onhome_tiles().get(self.inner_index.unwrap_or(0)).cloned() else { return };
        let Some(id) = tile.get("id").and_then(Value::as_str) else { return };
        self.sheet = Some(super::onhome::Sheet::asking(id));
        self.open_layer(Layer::OnHome, SCOPE_ONHOME);
        self.redraw(out);
    }

    fn sheet_key(&mut self, name: &str, out: &mut Vec<Effect>) {
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
            "onhome" => self.onhome_tiles().len(),
            "homeassistant" => self.ha_items().len(),
            "cameras" => self.camera_items().len(),
            "news" => self.news_items().len(),
            _ => 0,
        };
        if n == 0 {
            return false;
        }
        let i = self.inner_index.unwrap_or(0);
        let cols = if kind == "onhome" { self.onhome_cols(wid) } else { 1 };
        match name {
            "left" => self.inner_index = Some(i.saturating_sub(1)),
            "right" => self.inner_index = Some((i + 1).min(n - 1)),
            "up" => self.inner_index = Some(if i >= cols { i - cols } else { i }),
            "down" => self.inner_index = Some(if i + cols < n { i + cols } else { i }),
            "ok" => {
                if kind == "onhome"
                    && let Some(tile) = self.onhome_tiles().get(i).cloned()
                {
                    // OK does the tile's one action; a tile with finer controls, or only shown, opens its sheet
                    let can: Vec<&str> = tile.get("can").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                    let id = tile.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
                    match can.as_slice() {
                        ["toggle"] => self.send_act(&super::onhome::Act { id, verb: "toggle", value: Value::Null }, out),
                        ["run"] => self.send_act(&super::onhome::Act { id, verb: "run", value: Value::Null }, out),
                        // an open lock is locked at once; a locked one asks before it unlocks
                        ["lock"] => self.send_act(&super::onhome::Act { id, verb: "lock", value: Value::Null }, out),
                        ["unlock"] => self.open_unlock_question(out),
                        _ => self.open_sheet(out),
                    }
                }
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
        }
        if self.layer == Layer::Popover
            && let Some(p) = self.popover.take()
        {
            self.keep_popover(p);
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
        self.search.open(apps, self.voice_ready, self.now);
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
        if let Some(first) = self.cards().first().map(|e| e.id.clone())
            && let Some(id) = self.widget_id(&ed.sel).or_else(|| self.widget_id(&first))
        {
            self.focus.focus(id);
        }
        self.redraw(out);
    }

    fn edit_key(&mut self, name: &str, dir: Option<Dir>, out: &mut Vec<Effect>) {
        let Some(ed) = &mut self.editor else { return };
        let outs = ed.key(name, dir, &mut self.focus, self.now);
        for o in outs {
            match o {
                EditorOut::Preview(l) => self.preview_layout(l),
                EditorOut::Save(l) => {
                    let file = grid::for_file(&l, &self.shipped_layout);
                    let text = serde_json::to_string_pretty(&file).unwrap_or_default() + "\n";
                    self.layout_text = file.to_string();
                    out.push(Effect::Write { path: "theme/layout.json".into(), text });
                    out.push(Effect::Log(format!("layout.json written, {} cards", grid::widgets(&file).len())));
                    self.apply_layout(file);
                    self.edit_end(out);
                }
                EditorOut::Leave(orig) => {
                    self.apply_layout(serde_json::from_str(&self.layout_text).unwrap_or_else(|_| json!({})));
                    let _ = orig;
                    self.edit_end(out);
                }
            }
        }
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

    fn search_key(&mut self, name: &str, dir: Option<Dir>, out: &mut Vec<Effect>) {
        let o = self.search.key(name, dir, &mut self.focus, self.now);
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
                    _ => out.push(Effect::Log(format!("search picked {kind} {id}"))),
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

    /// Where the panel and its rows are now (the surface draws what this says), and the rows in view registered as the pointer's targets.
    pub fn popover_layout(&mut self) -> Option<popover::Layout> {
        let item = self.popover.as_ref()?.item;
        let (sx, sy) = self.safe();
        let bar = ui::geometry::Frame::new(sx, sy, 1920.0 - 2.0 * sx, self.status_height());
        let &(x, y, w, h) = self.status_frames().get(item)?;
        let left = item < self.left_count();
        let scale = self.text_scale;
        self.popover.as_mut().map(|p| p.layout(bar, ui::geometry::Frame::new(x, y, w, h), left, scale))
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
            Done::Settings => {
                self.close_layer();
                self.requests.push(Request::Settings);
            }
            Done::TvSettings => {
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
        let globe = self.radar.open && self.radar.level() == super::radar_view::Level::Globe;
        if self.wview.page == super::weather_view::Page::Radar && (matches!(name, "left" | "right" | "ok" | "1" | "2" | "3") || (globe && matches!(name, "up" | "down"))) {
            let (fx, _) = self.radar.key(name, self.now);
            out.extend(fx);
            return;
        }
        if self.wview.key(name, dir == Some(Dir::Up), dir == Some(Dir::Down)) == super::weather_view::Done::Close {
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
                out.extend(self.radar.open(lat, lon, super::radar_view::Level::Home, !self.light, self.now));
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
            return;
        }
        self.cursor_visible = true;
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
            if let Some(id) = self.focus.hit(x, y).filter(|id| self.focus.is_live(*id)) {
                self.focus.click(x, y);
                self.click_press = Some((id, self.now));
            }
            return;
        }
        if !self.pointer_down && was {
            let landed = self.click_press.take().map(|(id, _)| id);
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
        let h = self.focus.hover(x, y, true);
        if h.from != h.to {
            self.redraw(out);
        }
    }

    fn edit_outs(&mut self, outs: Vec<EditorOut>, out: &mut Vec<Effect>) {
        for o in outs {
            match o {
                EditorOut::Preview(l) => self.preview_layout(l),
                EditorOut::Save(l) => {
                    let file = grid::for_file(&l, &self.shipped_layout);
                    let text = serde_json::to_string_pretty(&file).unwrap_or_default() + "\n";
                    self.layout_text = file.to_string();
                    out.push(Effect::Write { path: "theme/layout.json".into(), text });
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

    /// Whether the weather card's picture may move: Home in front with the screen on, no screensaver over it, no layer over the cards and no
    /// full-screen modal over Home.
    pub fn weather_art_runs(&self) -> bool {
        self.active && self.screen_on && !self.saver_up && self.layer == Layer::None && !self.covered
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
            "onhome": json!({"tiles": self.onhome_tiles().len(), "index": self.inner_index, "sheet": self.sheet.as_ref().map(|s| s.id.clone()), "notice": self.onhome_notice()}),
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

    fn press(h: &mut Home, name: &str, at: u64) -> Vec<Effect> {
        let mut out = h.update(&Input::Key { name: name.into(), down: true, repeat: false }, at);
        out.extend(h.update(&Input::Key { name: name.into(), down: false, repeat: false }, at + 30));
        out
    }

    /// The helper's `plugins` topic as the TV answers it, with the roster of the helper's own plugins and the owner's.
    fn roster(owner: Value) -> Value {
        let own = |id: &str| json!({"class": "S", "firstParty": true, "id": id, "name": id, "registered": true, "state": "connected", "version": "1"});
        json!({"firstParty": [own("weather"), own("news"), own("clock")], "plugins": owner, "problem": null})
    }

    #[test]
    fn ok_on_the_weather_card_opens_the_weather_view_up_and_down_walk_it_and_back_closes() {
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
        press(&mut h, "up", 2100);
        assert_eq!(h.wview.page, super::super::weather_view::Page::Now, "nothing above the first page");
        press(&mut h, "down", 2200);
        assert_eq!(h.wview.page, super::super::weather_view::Page::Radar);
        assert!(h.radar.open || h.weather.pointer("/doc/lat").is_none(), "the helper's radar is asked for when the page comes (with a place to ask for)");
        press(&mut h, "down", 2250);
        assert_eq!(h.wview.page, super::super::weather_view::Page::Details);
        assert!(!h.radar.open, "and let go when the page goes");
        press(&mut h, "down", 2400);
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
    fn a_modal_over_home_rests_the_weather_and_a_panel_only_stills_the_wallpaper() {
        let mut h = home();
        assert!(h.weather_art_runs() && !h.wallpaper_held(), "Home alone: everything moves");
        assert!(h.set_overlay(false, true), "a panel is up: it changed");
        assert!(h.weather_art_runs(), "the weather card is beside a panel, in view");
        assert!(h.wallpaper_held(), "the wallpaper is held under a panel");
        assert!(!h.set_overlay(false, true), "the same again changes nothing, so no frame is asked for");
        assert!(h.set_overlay(true, true), "Settings is up");
        assert!(!h.weather_art_runs() && h.wallpaper_held(), "a modal covers the card: its sprites rest");
        assert!(h.set_overlay(false, false));
        assert!(h.weather_art_runs() && !h.wallpaper_held(), "back to Home alone");
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
        press(h, "down", 1100);
        for i in 0..8 {
            if h.focus.current().and_then(|id| h.widget_of(id)) == Some("smart") {
                break;
            }
            press(h, "down", 1200 + i * 100);
        }
        assert_eq!(h.focus.current().and_then(|id| h.widget_of(id)), Some("smart"), "the focus reached the smart-home card");
        press(h, "ok", 2000);
        assert_eq!(h.inner.as_deref(), Some("smart"), "OK on the card enters it");
    }

    fn act_of(out: &[Effect]) -> Option<Value> {
        out.iter().find_map(|e| match e {
            Effect::Request { tag, method, path, body } if *tag == ONHOME_TAG && method == "POST" && path == "/v1/onhome/act" => body.clone(),
            _ => None,
        })
    }

    #[test]
    fn the_smart_home_card_is_entered_with_ok_and_ok_on_a_tile_does_its_one_action() {
        let mut h = smart_home(json!([lamp(), thermostat(), sensor()]));
        into_smart_card(&mut h);
        assert_eq!(h.inner_index, None);
        // OK on a light turns it
        let out = press(&mut h, "ok", 2100);
        assert_eq!(act_of(&out), Some(json!({"id": "light.lamp", "verb": "toggle", "value": null})));
        assert_eq!(h.layer, Layer::None);
        // the keys move over the tiles two to a row on a card this wide
        press(&mut h, "right", 2200);
        assert_eq!(h.inner_index, Some(1));
        press(&mut h, "down", 2300);
        assert_eq!(h.inner_index, Some(1), "a second row has only one more tile: down from the right column stays");
        press(&mut h, "left", 2400);
        press(&mut h, "down", 2500);
        assert_eq!(h.inner_index, Some(2), "down is a row on");
        press(&mut h, "up", 2600);
        assert_eq!(h.inner_index, Some(0));
        // a thermostat opens its sheet on OK; a thing that is only shown opens a sheet of words
        press(&mut h, "right", 2700);
        let out = press(&mut h, "ok", 2800);
        assert!(act_of(&out).is_none());
        assert_eq!((h.layer, h.sheet.as_ref().map(|s| s.id.as_str())), (Layer::OnHome, Some("climate.t")));
        assert_eq!(h.state()["layer"], "onhome");
        press(&mut h, "back", 2900);
        assert_eq!((h.layer, h.sheet.is_none()), (Layer::None, true));
        assert_eq!(h.inner.as_deref(), Some("smart"), "Back from the sheet returns to the tiles");
        press(&mut h, "back", 3000);
        assert!(h.inner.is_none());
    }

    #[test]
    fn a_held_ok_or_the_menu_key_on_a_tile_opens_its_controls_and_left_and_right_set_a_value_that_is_sent_once_it_rests() {
        let mut h = smart_home(json!([lamp(), thermostat(), sensor()]));
        into_smart_card(&mut h);
        press(&mut h, "right", 2100);
        press(&mut h, "left", 2150);
        h.update(&Input::Key { name: "ok".into(), down: true, repeat: false }, 3000);
        let out = h.update(&Input::Tick, 3800);
        assert!(act_of(&out).is_none(), "a hold is not an OK");
        assert_eq!((h.layer, h.sheet.as_ref().map(|s| s.id.as_str())), (Layer::OnHome, Some("light.lamp")));
        h.update(&Input::Key { name: "ok".into(), down: false, repeat: false }, 3850);
        assert_eq!(h.layer, Layer::OnHome, "the release of the hold does not press a row");
        // down to Brightness, two steps right
        press(&mut h, "down", 4000);
        let a = act_of(&press(&mut h, "right", 4100));
        let b = act_of(&press(&mut h, "right", 4200));
        assert_eq!((a, b), (None, None), "a slider is not sent while the key moves");
        let out = h.update(&Input::Tick, 4700);
        assert_eq!(act_of(&out), Some(json!({"id": "light.lamp", "verb": "brightness", "value": 70.0})));
        press(&mut h, "back", 4800);
        press(&mut h, "back", 4900);
        // the Menu key is the same as the hold
        press(&mut h, "ok", 5000);
        assert!(h.inner.is_some());
        press(&mut h, "menu", 5100);
        assert_eq!(h.layer, Layer::OnHome);
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
        press(&mut h, "down", 1200);
        for i in 0..8 {
            if h.focus.current().and_then(|id| h.widget_of(id)) == Some("smart") {
                break;
            }
            press(&mut h, "down", 1300 + i * 100);
        }
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
        assert_eq!(h.state()["widgetFocusId"], "clock");
        press(&mut h, "right", 2100);
        assert_eq!(h.state()["widgetFocusId"], "news");
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
    fn ok_on_a_tile_launches_and_a_digit_opens_a_tile_and_apps_opens_the_library() {
        let mut h = home();
        press(&mut h, "down", 2000);
        press(&mut h, "down", 2100);
        press(&mut h, "down", 2200);
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
        assert_eq!(h.editor.as_ref().map(|e| e.sel.as_str()), Some("clock"));
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
        assert_eq!(widths[0], Chip::new(48.0, Some(50.0)).width, "the chip is the first button, as wide as its parts");
        h.set_text_scale(2.0);
        assert_eq!(h.name_to_measure().as_deref(), Some("Nitin"), "a text size has its own width");
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
        assert_eq!(h.search.text, "net");
        assert_eq!(h.search.answers()[0].id, "netflix");
        h.update(&Input::Topic { topic: "keyboard.text".into(), data: json!({"text": "you"}) }, 2100);
        assert_eq!(h.search.text, "you");
        let out = h.update(&Input::Topic { topic: "keyboard.text".into(), data: json!({"text": "you\n"}) }, 2200);
        assert_eq!(h.layer, Layer::None, "Done closes the sheet");
        assert!(!out.iter().any(|e| matches!(e, Effect::Luna { .. })), "Done on a search opens nothing by itself");
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
        press(&mut h, "down", 2000);
        press(&mut h, "down", 2100);
        press(&mut h, "down", 2200);
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
        // the arrows reach the answer: the keyboard's top row is under them
        for _ in 0..5 {
            if h.state()["search"]["index"].as_u64().is_some_and(|i| i >= 43) {
                break;
            }
            press(&mut h, "up", 2200);
        }
        assert!(h.state()["search"]["index"].as_u64().unwrap() >= 43, "{}", h.state()["search"]);
        while h.state()["search"]["index"].as_u64() != Some(44) {
            press(&mut h, "down", 2300);
            assert!(h.state()["search"]["index"].as_u64().unwrap() >= 43);
        }
        let out = press(&mut h, "ok", 2400);
        assert_eq!(h.take_requests(), vec![Request::SettingsPage("network".into())]);
        assert_eq!(h.layer, Layer::None, "the sheet is closed behind it");
        assert!(!out.iter().any(|e| matches!(e, Effect::Luna { .. })), "nothing is launched");
        // a page picked is not an app: the next Search starts from the pages the shell gave
        press(&mut h, "search", 2600);
        assert!(h.take_pages_wanted() && h.search.answers().is_empty());
    }

    fn gear(h: &mut Home) {
        for i in 0..4 {
            if h.focus.current() == Some(Id(STATUS_ID + 2)) {
                break;
            }
            press(h, "right", 1500 + i * 100);
        }
        press(h, "ok", 2000);
    }

    #[test]
    fn ok_on_the_gear_opens_the_wallpaper_popover_whose_values_change_at_once_and_are_written_to_the_theme_when_it_closes() {
        let mut h = home();
        gear(&mut h);
        assert_eq!(h.layer, Layer::Popover);
        assert!(h.take_requests().is_empty(), "the gear opens the popover first, Settings is a row of it");
        press(&mut h, "down", 2100);
        press(&mut h, "down", 2200);
        assert_eq!(h.state()["popover"]["row"], "amplitude");
        let out = press(&mut h, "right", 2300);
        assert_eq!(h.theme["wallpaperMotion"]["amplitude"], 5.0, "Home behind has it at once");
        assert!(out.contains(&Effect::Redraw));
        assert_eq!(h.state()["popover"]["values"][2], "5 px");
        press(&mut h, "right", 2400);
        press(&mut h, "up", 2500);
        press(&mut h, "ok", 2600);
        assert_eq!(h.theme["wallpaperMotion"]["enabled"], false, "OK on the toggle turns it");
        let out = press(&mut h, "back", 2700);
        assert_eq!(h.layer, Layer::None);
        assert_eq!(h.focus.current(), Some(Id(STATUS_ID + 2)), "the gear has the focus again");
        let patch = out.iter().find_map(|e| match e {
            Effect::Theme { tag: POPOVER_TAG, patch } => Some(patch.clone()),
            _ => None,
        });
        assert_eq!(patch, Some(json!({"wallpaperMotion": {"amplitude": 6.0, "enabled": false}})), "what was changed, once, at the close");
        assert!(!press(&mut h, "down", 2900).iter().any(|e| matches!(e, Effect::Theme { tag: POPOVER_TAG, .. })));
    }

    #[test]
    fn the_rows_that_do_something_open_the_settings_kinds_page_or_lgs_settings_and_a_click_is_the_row_it_is_on_and_outside_it_closes() {
        let mut h = home();
        gear(&mut h);
        press(&mut h, "ok", 2100);
        assert_eq!((h.layer, h.take_requests()), (Layer::None, vec![Request::Settings]), "All settings");
        gear(&mut h);
        for i in 0..13 {
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
        click(&mut h, centre(&rows[3].1), 3000);
        assert_eq!(h.theme["wallpaperMotion"]["speed"], 1.1, "Speed took a step up");
        click(&mut h, centre(&rows[1].1), 3100);
        assert_eq!(h.theme["wallpaperMotion"]["enabled"], false);
        assert_eq!(h.layer, Layer::Popover, "a row's click leaves it up");
        click(&mut h, (200.0, 900.0), 3200);
        assert_eq!(h.layer, Layer::None, "a click elsewhere puts it away");
    }
}
