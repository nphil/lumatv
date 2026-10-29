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
pub const WIDGET_ID: u32 = 200;
pub const DOCK_ID: u32 = 300;
pub const DOCK_MAX: u32 = 60;

pub const SCOPE_APPS: Scope = Scope(1);
pub const SCOPE_SEARCH: Scope = Scope(2);
pub const SCOPE_EDIT: Scope = Scope(3);
pub const SCOPE_PICKER: Scope = Scope(4);
pub const SCOPE_MENU: Scope = Scope(6);

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
    /// The status bar's settings popover: the wallpaper's motion in live rows ([`super::popover`]).
    Popover,
}

/// A row of the tile menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub id: &'static str,
    pub label: String,
    pub danger: bool,
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

pub struct Home {
    pub theme: Value,
    pub layout: Value,
    pub shipped_layout: Value,
    pub layout_text: String,
    pub theme_text: String,
    pub tint: Value,
    pub weather: Value,
    pub news: Value,
    /// The helper's `plugins` roster, its `ha.link` and the plugins' `plugin.<id>.cards`: what the At-home card is drawn from.
    pub plugins: hub::Topics,
    pub cameras: Value,
    pub stills: super::stills::Stills,
    /// What the bell's badge counts (the panels' log and the plugins' notes since the drawer was last open).
    pub unread: usize,
    pub profiles: Value,
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
    /// The status bar's settings popover while it is up, and what it changed that is to be written to the theme at the next update.
    pub popover: Option<Popover>,
    /// Where the popover and its rows are (made when it opens and when its focus moves; the surface draws this).
    pub popover_lay: Option<popover::Layout>,
    persist: Option<Value>,
    pub inner: Option<String>,
    pub inner_index: Option<usize>,
    pointer_down: bool,
    pub active: bool,
    /// The clip of the moving wallpaper is on the plane under Home and the hole is open: Home's own picture and scrim are away ([`WallVideo::up`]).
    pub wall_hole: bool,
    /// What the compositor's `wall` topic says: the hole of the wallpaper's window is under Home.
    wall_topic: bool,
    /// The moving wallpaper's controller: asks the player for the clip while Home wants it.
    pub wall: WallVideo,
    light: bool,
    saver_up: bool,
    pub screen_on: bool,
    pub cursor_visible: bool,
    pub launching: Option<String>,
    ok_down_at: Option<u64>,
    ok_held: bool,
    moved_at: u64,
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
            news: Value::Null,
            plugins: Default::default(),
            voice_ready: false,
            cameras: Value::Null,
            stills: super::stills::Stills::default(),
            unread: 0,
            profiles: Value::Null,
            inner: None,
            inner_index: None,
            pointer_down: false,
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
            popover: None,
            popover_lay: None,
            persist: None,
            active: false,
            wall_hole: false,
            wall_topic: false,
            wall: WallVideo::default(),
            light: false,
            saver_up: false,
            screen_on: true,
            cursor_visible: false,
            launching: None,
            ok_down_at: None,
            ok_held: false,
            moved_at: 0,
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

    /// The status bar's buttons sit in a pill at the safe corner: item `i` is `48 + 6` apart, left from 6, right ending 10 in.
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
        let mut rx = sx + w - 10.0 - total;
        for iw in right {
            out.push((rx, sy + 6.0, iw, item_h));
            rx += iw + 6.0;
        }
        out
    }

    /// A button is a square of the pill's inner height; the profile chip adds its name.
    pub fn status_item_width(&self, item: &Value) -> f32 {
        let h = self.status_height() - 12.0;
        if item.get("type").and_then(Value::as_str) == Some("profile") {
            let name = self.profile_name();
            if name.is_empty() { h } else { h + 12.0 + 15.0 * self.text_scale.max(1.0) * name.chars().count() as f32 + 16.0 }
        } else {
            h
        }
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
        [1920.0 - sx - 10.0 - side, sy + 6.0, side, side]
    }

    // ---- profiles
    pub fn profile(&self) -> Option<&Value> {
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
    }

    pub fn due_at(&self) -> Option<u64> {
        let mut due: Option<u64> = None;
        let mut take = |t: Option<u64>| {
            if let Some(t) = t {
                due = Some(due.map_or(t, |d| d.min(t)));
            }
        };
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
        // the clock's minute, and its second hand when the analogue face shows one
        let step = if self.theme.pointer("/clock/style").and_then(Value::as_str) == Some("analog") { 1000 } else { 60_000 };
        take(Some((self.now / step + 1) * step));
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
            "news" => self.news = data.clone(),
            "plugins" | "ha.link" => {
                self.plugins.insert(topic.to_owned(), data.clone());
            }
            t if t.starts_with("plugin.") && t.ends_with(".cards") => {
                self.plugins.insert(topic.to_owned(), data.clone());
            }
            "cameras" => self.cameras = data.clone(),
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
            TINT_TAG if status == 200 => {
                self.tint = data.clone();
                self.redraw(out);
            }
            SWITCH_TAG if self.picker.answered(status, data, self.now) => self.redraw(out),
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
            Input::Pointer { .. } => self.pointer_down = false,
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
        let dir = Self::dir_of(name);
        match self.layer {
            Layer::Apps => self.apps_key(name, dir, out),
            Layer::Search => self.search_key(name, dir, out),
            Layer::Edit => self.edit_key(name, dir, out),
            Layer::Picker => self.picker_key(name, dir, out),
            Layer::Menu => self.menu_key(name, dir, out),
            Layer::Popover => self.popover_key(name, dir, out),
            Layer::None => self.home_key(name, dir, out),
        }
        self.redraw(out);
    }

    fn home_key(&mut self, name: &str, dir: Option<Dir>, out: &mut Vec<Effect>) {
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
            _ => false,
        }
    }

    /// A key inside a card: the content's rows. Returns false when the content gives the keys back.
    fn inner_key(&mut self, wid: &str, name: &str, out: &mut Vec<Effect>) -> bool {
        let kind = self.cards().iter().find(|e| e.id == wid).map(|e| e.kind.clone()).unwrap_or_default();
        let n = match kind.as_str() {
            "homeassistant" => self.ha_items().len(),
            "cameras" => self.camera_items().len(),
            "news" => self.news_items().len(),
            _ => 0,
        };
        if n == 0 {
            return false;
        }
        let i = self.inner_index.unwrap_or(0);
        match name {
            "up" | "left" => self.inner_index = Some(i.saturating_sub(1)),
            "down" | "right" => self.inner_index = Some((i + 1).min(n - 1)),
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
        if self.layer == Layer::Popover
            && let Some(p) = self.popover.take()
        {
            self.keep_popover(p);
            self.popover_lay = None;
            for i in 0..popover::ROWS.len() as u32 {
                self.focus.unregister(Id(popover::ROW_ID + i));
            }
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
            let app = json!({"id": p.id, "title": p.title, "iconUrl": p.icon, "params": p.params, "onDock": true, "unmovable": p.unmovable});
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
        if !unmovable && app.get("id").and_then(Value::as_str).is_some_and(|i| !i.starts_with("com.webos.")) {
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
                    "uninstall" => {
                        out.push(Effect::Luna { tag: LAUNCH_TAG, uri: "luna://com.webos.appInstallService/remove".into(), payload: json!({"id": app_id}), subscribe: false });
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
            return;
        }
        self.cursor_visible = true;
        let was = self.pointer_down;
        self.pointer_down = buttons & 1 != 0;
        if self.pointer_down && !was {
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
            if let Some(id) = self.focus.click(x, y) {
                if self.layer == Layer::Apps {
                    let action = self.apps.clicked(id, &mut self.focus);
                    self.apps_out(action, out);
                    self.redraw(out);
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
            "layer": match self.layer { Layer::None => "", Layer::Apps => "apps", Layer::Search => "search", Layer::Edit => "edit", Layer::Picker => "picker", Layer::Menu => "menu", Layer::Popover => "popover" },
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
            lps.push(json!({"id": id, "title": t, "icon": format!("/{id}.png"), "iconColor": "#101010"}));
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
        assert!(out.iter().any(|e| matches!(e, Effect::Luna { payload, .. } if payload["id"] == "netflix")), "a click opens what is under it");
        h.update(&Input::Pointer { x, y, buttons: 0, visible: true }, 2200);
        h.update(&Input::Key { name: "left".into(), down: true, repeat: false }, 2300);
        assert_eq!(h.focus.hovered(), None, "the next key hides the cue");
    }

    fn notes_of(h: &mut Home) -> Vec<Note> {
        h.take_notes()
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
