//! The quick panel (`docs/mocks/10-quick-panel`, `panels/QuickPanel.qml`, `QuickList.qml`, `QuickEdit.qml`, `QuickPopup.qml` of the QML
//! shell): a tall sheet docked to the right edge, 560 wide and 40 from it, with the few things reached for most, one row each. Left
//! and Right change a value, OK opens the page that has more. The profiles' strip is its first row (one OK switches; a profile that has
//! a PIN asks it in place), "All settings" is under the list, one Down past the last tile. OK on a stepper, or on a tile that has a
//! card, opens a card beside the panel (a value picker, a slider), and a held OK on a tile edits the tiles: move one, take one off, add
//! one back from the tiles the panel does not show. The owner's order and choice is the theme's `quickPanel.tiles` ([`arrange`]).
//!
//! This is the model: what is focused, what a key does, what the rows say. The surface draws [`Quick::view`].

pub mod tiles;
pub(crate) mod info;
pub use info::Display as DisplayInfo;

use super::arrange;
use super::clock;
use super::env::{Env, tags};
use super::hold::Hold;
use super::life::Life;
use super::pin::{self, Pin, Verdict};
use super::{Ask, K, Out};
use crate::kit::focus::Id;
use crate::kit::hint::{Action, Hint, Key};
use crate::kit::screen::{Effect, Input};
use serde_json::{Value, json};
use tiles::{Act, Kind, Row, Slider};

/// Where the keys are: the profiles' strip above the list, the list, or "All settings" under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zone {
    Strip,
    List,
    Foot,
}

/// What a pointer can land on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Strip(usize),
    Row(usize),
    Foot,
    Option(usize),
    EditRow(usize),
    EditMove(usize, i32),
    EditRemove(usize),
    EditAdd,
    Pad(usize),
    /// Nothing: the screen behind the panel (a click on it is Back).
    Backdrop,
}

impl Target {
    const BASE: u32 = 0x2000_0000;

    /// The focusable's name in the engine's registry.
    pub fn id(self) -> Id {
        let (kind, i) = match self {
            Target::Strip(i) => (1, i),
            Target::Row(i) => (2, i),
            Target::Foot => (3, 0),
            Target::Option(i) => (4, i),
            Target::EditRow(i) => (5, i),
            Target::EditMove(i, -1) => (6, i),
            Target::EditMove(i, _) => (7, i),
            Target::EditRemove(i) => (8, i),
            Target::EditAdd => (9, 0),
            Target::Pad(i) => (10, i),
            Target::Backdrop => (11, 0),
        };
        Id(Self::BASE | (kind << 16) | i as u32)
    }

    pub fn from_id(id: Id) -> Option<Target> {
        if id.0 & 0xFF00_0000 != Self::BASE & 0xFF00_0000 || id.0 & 0x00F0_0000 != 0 {
            return None;
        }
        let i = (id.0 & 0xFFFF) as usize;
        Some(match (id.0 >> 16) & 0xFF {
            1 => Target::Strip(i),
            2 => Target::Row(i),
            3 => Target::Foot,
            4 => Target::Option(i),
            5 => Target::EditRow(i),
            6 => Target::EditMove(i, -1),
            7 => Target::EditMove(i, 1),
            8 => Target::EditRemove(i),
            9 => Target::EditAdd,
            10 => Target::Pad(i),
            11 => Target::Backdrop,
            _ => return None,
        })
    }
}

/// A value to an act: the index a stepper chose, the way a nudge was pushed, a switch's new state, a slider's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Val {
    None,
    Index(usize),
    Delta(i32),
    On(bool),
    Num(f64),
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Edit {
    /// 0 to `rows.len() - 1` a tile, `rows.len()` "Add a tile".
    index: usize,
    grab: bool,
    on_remove: bool,
    undo: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq)]
enum Popup {
    Slider { row: String, spec: Slider, shown: f64, key_at: u64 },
    Pick { row: String, act: Act, title: String, text: &'static str, icon: String, options: Vec<String>, index: usize },
    Add { items: Vec<(String, String, String)>, text: &'static str, index: usize },
}

/// A profile as the strip shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    pub id: String,
    pub name: String,
    pub initial: String,
    pub color: String,
    pub avatar_file: Option<String>,
    pub active: bool,
    pub locked: bool,
    pub owner: bool,
}

pub fn people(env: &Env) -> Option<Vec<Person>> {
    let t = env.topic("profiles")?;
    if t.get("ready").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let list: Vec<Person> = t
        .get("profiles")
        .and_then(Value::as_array)?
        .iter()
        .map(|p| {
            let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            Person {
                id: s("id"),
                name: s("name"),
                initial: s("initial"),
                color: s("color"),
                avatar_file: p.get("avatarFile").and_then(Value::as_str).map(str::to_string),
                active: p.get("active").and_then(Value::as_bool) == Some(true),
                locked: p.get("locked").and_then(Value::as_bool) == Some(true),
                owner: p.get("owner").and_then(Value::as_bool) == Some(true),
            }
        })
        .collect();
    // one person needs no strip: the status bar's chip names them
    (list.len() > 1).then_some(list)
}

/// Whether the owner's profile is the active one (a PIN is not asked of the owner).
fn owner_active(env: &Env) -> bool {
    env.topic("profiles").is_some_and(|t| t.get("active") == t.get("owner") && t.get("active").is_some())
}

#[derive(Clone, Debug, PartialEq)]
pub enum Focus {
    Strip(usize),
    Row(usize),
    Foot,
    EditRow(usize),
    EditAdd,
    Option(usize),
    Pad(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditView {
    pub rows: Vec<Row>,
    pub index: usize,
    pub grab: bool,
    pub on_remove: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptionView {
    pub label: String,
    pub icon: String,
    pub detail: String,
    pub current: bool,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PopupView {
    Slider { title: String, text: String, from: f64, to: f64, step: f64, value: f64, marks: [&'static str; 3], icons: [&'static str; 2] },
    Pick { title: String, text: String, options: Vec<OptionView>, index: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PadView {
    pub avatar: Option<Person>,
    pub pin: pin::View,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoInfo { pub width: Option<u64>, pub height: Option<u64>, pub fps: Option<f64> }

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub up: bool,
    pub editing: bool,
    pub title: String,
    /// The head's second word: the time ("9:52 PM"), empty in edit mode where the glyphs say what the keys do.
    pub clock: String,
    pub info: DisplayInfo,
    pub video_info: Option<VideoInfo>,
    pub hints_head: Vec<Hint>,
    pub hints_foot: Vec<Hint>,
    pub strip: Option<Vec<Person>>,
    pub pad: Option<PadView>,
    pub rows: Vec<Row>,
    pub edit: Option<EditView>,
    pub popup: Option<PopupView>,
    pub focus: Option<Focus>,
    pub popups_on: bool,
    pub over_video: bool,
}

pub struct Quick {
    pub life: Life,
    zone: Zone,
    stop: Option<usize>,
    seg: usize,
    touched: bool,
    editing: bool,
    edit: Edit,
    popup: Option<Popup>,
    focus_id: String,
    strip: usize,
    pin: Option<Pin>,
    /// The owner's arrangement as known here (shown at once, written when final); `None` is the panel's own list.
    order: Option<Vec<String>>,
    persisted: Option<Vec<String>>,
    read_due: Option<u64>,
    slider_due: Option<u64>,
    clock_due: Option<u64>,
    shot_due: Option<u64>,
    info_due: Option<u64>,
    info_tag: u64,
    info_answered: Option<u64>,
    video: Option<info::Video>,
    info_authority: Option<(String, String)>,
    monitor: info::Monitor,
    pub hold: Hold,
}

impl Default for Quick {
    fn default() -> Self {
        Self::new()
    }
}

/// How long after it opens the panel waits to ask the TV for its readings: after the slide and the burst of frames the key started.
pub const READ_AFTER_MS: u64 = 200;

impl Quick {
    pub fn new() -> Self {
        Self {
            life: Life::default(),
            zone: Zone::List,
            stop: None,
            seg: 0,
            touched: false,
            editing: false,
            edit: Edit::default(),
            popup: None,
            focus_id: String::new(),
            strip: 0,
            pin: None,
            order: None,
            persisted: None,
            read_due: None,
            slider_due: None,
            clock_due: None,
            shot_due: None,
            info_due: None,
            info_tag: 0x5A00_0000,
            info_answered: None,
            video: None,
            info_authority: None,
            monitor: info::Monitor::default(),
            hold: Hold::default(),
        }
    }

    // ---- the rows

    /// The tiles as the panel shows them: the owner's arrangement, or its own list.
    pub fn rows(&self, env: &Env) -> Vec<Row> {
        let all = tiles::everything(env);
        let ids = arrange::shown(self.order.as_deref(), &tiles::catalogue(&all));
        ids.iter().filter_map(|id| all.iter().find(|r| r.id == *id).cloned()).collect()
    }

    fn ids(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.id.clone()).collect()
    }

    fn first(rows: &[Row]) -> Option<usize> {
        rows.iter().position(Row::focusable)
    }

    fn step(rows: &[Row], from: usize, dir: i32) -> Option<usize> {
        let mut i = from as i32 + dir;
        while i >= 0 && (i as usize) < rows.len() {
            if rows[i as usize].focusable() {
                return Some(i as usize);
            }
            i += dir;
        }
        None
    }

    fn enter(&mut self, rows: &[Row], i: Option<usize>) {
        self.stop = i;
        self.seg = match i.and_then(|i| rows.get(i)) {
            Some(r) if r.kind == Kind::Stepper => r.index.unwrap_or(0).min(r.options.len().saturating_sub(1)),
            _ => 0,
        };
    }

    // ---- opening and closing

    /// The panel is wanted: it starts over (the focus on the first tile, the list, no card), the receiver's gate looks again, and the
    /// readings are asked for once the slide is over.
    pub fn open(&mut self, env: &mut Env, now: u64) -> Out {
        self.zone = Zone::List;
        self.touched = false;
        self.editing = false;
        self.popup = None;
        self.pin = None;
        self.strip = 0;
        self.hold.clear();
        self.order = arrange::saved(&env.theme);
        self.persisted = self.order.clone();
        let rows = self.rows(env);
        let first = Self::first(&rows);
        self.stop = None;
        self.enter(&rows, first);
        self.sync_focus_id(&rows);
        self.read_due = Some(now + u64::from(self.life.enter_ms) + READ_AFTER_MS);
        self.clock_due = Some(now + 15_000);
        self.video = None;
        self.info_authority = env.screen_on.then(|| env.fg.video.clone()).flatten();
        self.info_answered = None;
        self.info_tag = self.info_tag.wrapping_add(1);
        self.monitor.set_active(false);
        self.info_due = Some(now + u64::from(self.life.enter_ms) + READ_AFTER_MS);
        self.monitor.set_active(env.screen_on);
        Out::effects(env.gate.set_active(true, now))
    }

    /// The panel is going: a change only on the screen so far is written, the readings stop.
    pub fn close(&mut self, env: &mut Env, now: u64) -> Out {
        let mut out = self.end_edit();
        self.popup = None;
        self.pin = None;
        self.read_due = None;
        self.clock_due = None;
        self.info_due = None;
        self.info_answered = None;
        self.video = None;
        self.info_authority = None;
        self.monitor.set_active(false);
        self.hold.clear();
        out.effects.extend(env.gate.set_active(false, now));
        out
    }

    fn sync_focus_id(&mut self, rows: &[Row]) {
        if let Some(r) = self.stop.and_then(|i| rows.get(i)) {
            self.focus_id = r.id.clone();
        }
    }

    /// The services answered, or the tiles changed (a plugin's came or went): the focus stays on the same tile by id, and until a key is
    /// pressed it goes to the first tile that can take it (the picture mode is only focusable once the settings service has answered).
    pub fn sync(&mut self, env: &Env) {
        if self.info_due.is_some() {
            let authority = if env.screen_on { env.fg.video.as_ref() } else { None };
            if self.info_authority.as_ref() != authority {
                self.info_authority = authority.cloned();
                self.info_tag = self.info_tag.wrapping_add(1);
                self.info_answered = None;
                self.video = None;
                self.monitor.reset();
            }
            self.monitor.set_active(env.screen_on);
        }
        if !self.life.presented() {
            return;
        }
        let rows = self.rows(env);
        if !self.touched && !self.editing {
            let first = Self::first(&rows);
            if self.stop != first {
                self.enter(&rows, first);
            }
            self.sync_focus_id(&rows);
            return;
        }
        if let Some(i) = rows.iter().position(|r| r.id == self.focus_id && r.focusable()) {
            if self.stop != Some(i) {
                self.stop = Some(i);
            }
        } else if !self.focus_id.is_empty() && !rows.iter().any(|r| r.id == self.focus_id && r.focusable()) {
            let first = Self::first(&rows);
            self.enter(&rows, first);
            self.sync_focus_id(&rows);
        }
        if self.editing {
            self.edit.index = self.edit.index.min(rows.len());
        }
    }

    // ---- persisting the arrangement

    /// The owner's order, shown at once and (`keep`) written.
    fn keep_order(&mut self, order: Vec<String>, keep: bool) -> Vec<Effect> {
        self.order = Some(order.clone());
        if keep && self.persisted.as_ref() != Some(&order) {
            self.persisted = Some(order.clone());
            return vec![Effect::Theme { tag: tags::THEME, patch: arrange::patch(&order) }];
        }
        Vec::new()
    }

    /// Leaving edit mode (Back, or the panel closing): a change that is only on the screen so far is written.
    fn end_edit(&mut self) -> Out {
        if !self.editing {
            return Out::default();
        }
        self.editing = false;
        // an arrangement that is only on the screen so far is written
        Out::effects(match self.order.clone() {
            Some(order) => self.keep_order(order, true),
            None => Vec::new(),
        })
    }

    // ---- what an act does

    fn set_level(env: &mut Env, v: f64, now: u64) -> Vec<Effect> {
        env.sio.write("picture.backlight", &json!(v.round().clamp(0.0, 100.0) as i64), None, now)
    }

    /// Carries out the act of the row `id`.
    fn act(&mut self, act: &Act, v: Val, env: &mut Env, now: u64) -> Out {
        let mut out = Out::default();
        match (act, v) {
            (Act::Picture(values), Val::Index(i)) => {
                if let Some((value, true)) = values.get(i) {
                    let available = env.sio.list("picture.pictureMode").map_or_else(
                        || env.sio.desc("picture.pictureMode").is_none() && tiles::MODE_VALUES.contains(&value.as_str()) && env.sio.picture_dynamic_range().is_some_and(|range| range.eq_ignore_ascii_case("sdr")),
                        |list| list.iter().any(|item| item.value == *value && item.visible && item.active),
                    );
                    if available && env.sio.usable("picture.pictureMode") {
                        out.effects = env.sio.write("picture.pictureMode", &json!(value), None, now);
                    }
                }
            }
            (Act::Receiver, Val::Delta(d)) => {
                // the volume the way the keys move it, through the TV (which sends it to the receiver over HDMI-CEC); the helper is told
                // to read the receiver now so the new level appears at once
                let uri = if d > 0 { "luna://com.webos.service.audio/master/volumeUp" } else { "luna://com.webos.service.audio/master/volumeDown" };
                out.effects.push(Effect::Luna { tag: tags::VOLUME_NUDGE, uri: uri.into(), payload: json!({}), subscribe: false });
                out.effects.push(Effect::Post { path: "/v1/receiver/nudge".into(), body: json!({}) });
            }
            (Act::Receiver, Val::None) => {
                out.asks.push(Ask::Settings("receiver".into()));
                out.close = true;
            }
            (Act::Sleep, Val::Index(i)) => out.effects = env.power.set(i, now),
            (Act::Settings(page), _) => {
                out.asks.push(Ask::Settings((*page).to_string()));
                out.close = true;
            }
            (Act::Energy, Val::Index(i)) => out.effects = env.sio.write("picture.energySaving", &json!(tiles::ENERGY_VALUES[i.min(4)]), None, now),
            (Act::Oled, Val::Delta(d)) => out.effects = Self::set_level(env, env.sio.num("picture.backlight", 0.0) + f64::from(d) * 5.0, now),
            (Act::Oled, Val::Num(n)) => out.effects = Self::set_level(env, n, now),
            (Act::ScreenOff, _) => {
                out.effects = env.sio.write("option.screenOff", &json!("on"), None, now);
                out.close = true;
            }
            (Act::Pointer, Val::On(on)) => out.effects = env.sio.write("general.remotePointer", &json!(if on { "on" } else { "off" }), None, now),
            (Act::Eye, Val::On(on)) => out.effects = env.sio.write("picture.eyeComfortMode", &json!(if on { "on" } else { "off" }), None, now),
            (Act::Shot, _) => {
                // the screenshot is taken once the panel has slid away, so the picture is of what was under it
                self.shot_due = Some(now + 900);
                out.close = true;
            }
            (Act::Record, Val::On(on)) => {
                if on {
                    out.close = true;
                }
                out.effects.push(Effect::Request {
                    tag: tags::CAPTURE_POST,
                    method: "POST".into(),
                    path: if on { "/v1/capture/record/start" } else { "/v1/capture/record/stop" }.into(),
                    body: Some(json!({})),
                });
            }
            (Act::Plugin { plugin, action, target }, _) => {
                let body = target.as_ref().map_or_else(|| json!({}), |t| json!({"target": t}));
                out.effects.push(Effect::Post { path: format!("/v1/plugins/{plugin}/actions/{action}"), body });
            }
            _ => {}
        }
        out
    }

    /// OK on row `i`: a stepper's choices and a tile's own card open beside the panel when the owner has the cards on; a switch flips,
    /// a value or an action is pressed, a nudge opens its page.
    fn activate(&mut self, rows: &[Row], i: usize, env: &mut Env, now: u64) -> Out {
        let Some(r) = rows.get(i).filter(|r| r.focusable()).cloned() else { return Out::default() };
        if (r.slider.is_some() || r.kind == Kind::Stepper) && env.popups_on() {
            self.open_popup(&r, env, now);
            return Out::default();
        }
        match r.kind {
            Kind::Toggle => self.act(&r.act, Val::On(!r.on), env, now),
            Kind::Value | Kind::Action => self.act(&r.act, Val::None, env, now),
            Kind::Nudge if matches!(r.act, Act::Receiver) => self.act(&r.act, Val::None, env, now),
            _ => Out::default(),
        }
    }

    fn open_popup(&mut self, r: &Row, env: &Env, now: u64) {
        self.slider_due = Some(now + 400);
        self.popup = Some(if let Some(spec) = &r.slider {
            Popup::Slider { row: r.id.clone(), spec: spec.clone(), shown: env.sio.num("picture.backlight", 0.0), key_at: 0 }
        } else {
            let x = tiles::pick_extras(&r.id);
            Popup::Pick {
                row: r.id.clone(),
                act: r.act.clone(),
                title: r.label.clone(),
                text: x.text,
                icon: x.icon.map_or_else(|| r.icon.clone(), str::to_string),
                options: r.options.clone(),
                index: r.index.filter(|&i| !matches!(&r.act, Act::Picture(values) if !values.get(i).is_some_and(|(_, active)| *active))).or_else(|| match &r.act { Act::Picture(values) => values.iter().position(|(_, active)| *active), _ => None }).unwrap_or(0),
            }
        });
    }

    fn open_add(&mut self, env: &Env) {
        let all = tiles::everything(env);
        let shown = Self::ids(&self.rows(env));
        let left: Vec<(String, String, String)> = all.iter().filter(|r| !shown.contains(&r.id)).map(|r| (r.id.clone(), r.icon.clone(), r.label.clone())).collect();
        let text = if left.is_empty() { "Every tile is on the panel." } else { "Tiles not on the panel." };
        self.popup = Some(Popup::Add { items: left, text, index: 0 });
    }

    // ---- keys

    /// A key of the remote. `Out::close` asks the aggregator to hide the panel.
    pub fn key(&mut self, k: K, env: &mut Env, now: u64) -> Out {
        self.touched = true;
        if self.pin.is_some() {
            return self.pad_key(k, now);
        }
        if self.popup.is_some() {
            return self.popup_key(k, env, now);
        }
        if self.editing {
            return self.edit_key(k, env, now);
        }
        let rows = self.rows(env);
        match self.zone {
            Zone::Foot => {
                match k {
                    K::Up => self.zone = Zone::List,
                    K::Ok => {
                        let mut out = Out::default();
                        out.asks.push(Ask::Settings(String::new()));
                        out.close = true;
                        return out;
                    }
                    K::Back => return Out::close(),
                    _ => {}
                }
                Out::default()
            }
            Zone::Strip => self.strip_key(k, env, now),
            Zone::List => {
                // Down past the last tile lands on the button; the list itself stops at its last row
                if k == K::Down && self.stop.is_some_and(|s| Self::step(&rows, s, 1).is_none()) {
                    self.zone = Zone::Foot;
                    return Out::default();
                }
                if let Some(out) = self.list_key(k, &rows, env, now) {
                    self.sync_focus_id(&rows);
                    return out;
                }
                if k == K::Up && people(env).is_some() {
                    self.zone = Zone::Strip;
                    self.strip = people(env).and_then(|p| p.iter().position(|x| x.active)).unwrap_or(0);
                    return Out::default();
                }
                if k == K::Back { Out::close() } else { Out::default() }
            }
        }
    }

    /// The list's use of a key (`QuickList.key`): `None` is "not mine" (Up from the first tile, Left, Back).
    fn list_key(&mut self, k: K, rows: &[Row], env: &mut Env, now: u64) -> Option<Out> {
        if self.stop.is_none() {
            let first = Self::first(rows);
            self.enter(rows, first);
        }
        let stop = self.stop?;
        let r = rows.get(stop)?;
        match k {
            K::Up => {
                let p = Self::step(rows, stop, -1)?;
                self.enter(rows, Some(p));
                Some(Out::default())
            }
            K::Down => {
                if let Some(n) = Self::step(rows, stop, 1) {
                    self.enter(rows, Some(n));
                }
                Some(Out::default())
            }
            K::Left | K::Right => {
                let d = if k == K::Left { -1 } else { 1 };
                match r.kind {
                    Kind::Nudge => Some(self.act(&r.act.clone(), Val::Delta(d), env, now)),
                    Kind::Stepper => {
                        let from = if matches!(r.act, Act::Picture(_)) { r.index.unwrap_or(if d > 0 { r.options.len().saturating_sub(1) } else { 0 }) } else { self.seg };
                        if let Some(next) = r.step_index(from, d) {
                            self.seg = next;
                            Some(self.act(&r.act, Val::Index(next), env, now))
                        } else {
                            Some(Out::default())
                        }
                    }
                    _ => (d > 0).then(Out::default),
                }
            }
            K::Ok => Some(self.activate(rows, stop, env, now)),
            _ => None,
        }
    }

    fn strip_key(&mut self, k: K, env: &mut Env, now: u64) -> Out {
        let Some(list) = people(env) else {
            self.zone = Zone::List;
            return Out::default();
        };
        match k {
            K::Left => self.strip = self.strip.saturating_sub(1),
            K::Right => self.strip = (self.strip + 1).min(list.len() - 1),
            K::Down => self.zone = Zone::List,
            K::Ok => return self.switch_to(self.strip, env, now),
            K::Back => return Out::close(),
            _ => {}
        }
        Out::default()
    }

    /// OK on a profile: the current one is already; one that has a PIN asks it in place (the owner is never asked); the rest switch.
    fn switch_to(&mut self, i: usize, env: &mut Env, _now: u64) -> Out {
        let Some(p) = people(env).and_then(|l| l.get(i).cloned()) else { return Out::default() };
        if p.active {
            return Out::default();
        }
        if p.locked && !owner_active(env) {
            self.pin = Some(Pin::new(&p.id, &p.name, tags::PROFILE_SWITCH));
            return Out::default();
        }
        Out::effects(vec![Effect::Request { tag: tags::PROFILE_SWITCH, method: "POST".into(), path: "/v1/profiles/switch".into(), body: Some(json!({"id": p.id})) }])
    }

    fn pad_key(&mut self, k: K, now: u64) -> Out {
        let Some(pin) = self.pin.as_mut() else { return Out::default() };
        match pin.key(k, now) {
            pin::Step::Cancel => {
                self.pin = None;
                Out::default()
            }
            pin::Step::Submit(effects) => Out::effects(effects),
            pin::Step::None => Out::default(),
        }
    }

    fn popup_key(&mut self, k: K, env: &mut Env, now: u64) -> Out {
        let mut out = Out::default();
        let Some(mut popup) = self.popup.take() else { return out };
        let mut close = false;
        match &mut popup {
            Popup::Slider { row, spec, shown, key_at } => match k {
                K::Left | K::Right => {
                    let v = (*shown + if k == K::Left { -spec.step } else { spec.step }).clamp(spec.from, spec.to);
                    if v != *shown {
                        *shown = v;
                        *key_at = now;
                        let rows = tiles::everything(env);
                        if let Some(r) = rows.iter().find(|r| r.id == *row) { out = self.act(&r.act, Val::Num(v), env, now); }
                    }
                }
                K::Ok | K::Back => close = true,
                _ => {}
            },
            Popup::Pick { act, options, index, .. } => match k {
                K::Up | K::Down => {
                    let enabled = |i| !matches!(&*act, Act::Picture(values) if !values.get(i).is_some_and(|(_, active)| *active));
                    let next = if k == K::Up { (0..*index).rev().find(|&i| enabled(i)) } else { ((*index + 1)..options.len()).find(|&i| enabled(i)) };
                    if let Some(next) = next { *index = next; }
                }
                K::Ok => {
                    if !matches!(&*act, Act::Picture(values) if !values.get(*index).is_some_and(|(_, active)| *active)) {
                        out = self.act(act, Val::Index(*index), env, now);
                        close = true;
                    }
                }
                K::Back => close = true,
                _ => {}
            },
            Popup::Add { items, index, .. } => match k {
                K::Up => *index = index.saturating_sub(1),
                K::Down => *index = (*index + 1).min(items.len().saturating_sub(1)),
                K::Ok => {
                    close = true;
                    if let Some((id, _, _)) = items.get(*index) {
                        let shown = Self::ids(&self.rows(env));
                        let n = shown.len();
                        out.effects = self.keep_order(arrange::added(&shown, id), true);
                        self.edit.index = n;
                    }
                }
                K::Back => close = true,
                _ => {}
            },
        }
        if !close { self.popup = Some(popup); }
        out
    }

    fn set_option(&mut self, i: usize) {
        match self.popup.as_mut() {
            Some(Popup::Pick { index, .. }) | Some(Popup::Add { index, .. }) => *index = i,
            _ => {}
        }
    }

    // ---- edit mode (`QuickEdit.key`)

    /// A held OK on a tile means something where the owner has the editor on and a tile has the focus.
    pub fn hold_ready(&self, env: &Env) -> bool {
        self.life.presented() && env.popups_on() && !self.editing && self.popup.is_none() && self.pin.is_none() && self.zone == Zone::List && self.stop.is_some_and(|s| self.rows(env).get(s).is_some())
    }

    /// The hold fired: that tile is picked up in edit mode.
    pub fn held(&mut self, env: &Env) {
        if !self.hold_ready(env) {
            return;
        }
        let rows = self.rows(env);
        let at = self.stop.unwrap_or(0).min(rows.len().saturating_sub(1));
        self.editing = true;
        self.edit = Edit { index: at, grab: !rows.is_empty(), on_remove: false, undo: Some(Self::ids(&rows)) };
    }

    fn edit_key(&mut self, k: K, env: &mut Env, _now: u64) -> Out {
        let mut out = Out::default();
        let rows = self.rows(env);
        let ids = Self::ids(&rows);
        let add = rows.len();
        if self.edit.grab {
            match k {
                K::Up | K::Down => {
                    let d = if k == K::Up { -1 } else { 1 };
                    let j = self.edit.index as i32 + d;
                    if j >= 0 && (j as usize) < rows.len() {
                        let order = arrange::moved(&ids, &ids[self.edit.index], d);
                        self.edit.index = j as usize;
                        out.effects = self.keep_order(order, false);
                    }
                }
                K::Ok => {
                    self.edit.grab = false;
                    out.effects = self.keep_order(ids, true);
                }
                K::Back => {
                    // the tile goes back where it was and keeps the focus
                    let id = ids.get(self.edit.index).cloned().unwrap_or_default();
                    self.edit.grab = false;
                    if let Some(was) = self.edit.undo.clone() {
                        self.edit.index = was.iter().position(|x| *x == id).unwrap_or(0);
                        out.effects = self.keep_order(was, true);
                    }
                }
                _ => {}
            }
            return out;
        }
        match k {
            K::Up => {
                self.edit.index = self.edit.index.saturating_sub(1);
                self.edit.on_remove = false;
            }
            K::Down => {
                self.edit.index = (self.edit.index + 1).min(add);
                if self.edit.index == add {
                    self.edit.on_remove = false;
                }
            }
            K::Right => {
                if self.edit.index < add {
                    self.edit.on_remove = true;
                }
            }
            K::Left => self.edit.on_remove = false,
            K::Ok => {
                if self.edit.index == add {
                    self.open_add(env);
                } else if self.edit.on_remove {
                    out.effects = self.remove(&ids, self.edit.index);
                } else {
                    self.edit.grab = !rows.is_empty();
                    self.edit.on_remove = false;
                    self.edit.undo = Some(ids);
                }
            }
            K::Back => out = self.end_edit(),
            _ => {}
        }
        out
    }

    fn remove(&mut self, ids: &[String], i: usize) -> Vec<Effect> {
        let order: Vec<String> = ids.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, x)| x.clone()).collect();
        self.edit.grab = false;
        self.edit.on_remove = false;
        let n = order.len();
        let out = self.keep_order(order, true);
        // the next tile, or "Add a tile" when it was the last
        self.edit.index = i.min(n);
        out
    }

    // ---- the pointer

    /// A click on `t`: it takes the focus as the D-pad would and is acted on. `side` is -1 or 1 for a click on a stepper's left or right
    /// chevron (they step the value, as Left and Right do).
    pub fn click(&mut self, t: Target, side: Option<i32>, env: &mut Env, now: u64) -> Out {
        self.touched = true;
        if let Some(pin) = self.pin.as_mut() {
            return match t {
                Target::Pad(i) => match pin.click(i, now) {
                    pin::Step::Cancel => {
                        self.pin = None;
                        Out::default()
                    }
                    pin::Step::Submit(e) => Out::effects(e),
                    pin::Step::None => Out::default(),
                },
                _ => Out::default(),
            };
        }
        if let Some(popup) = self.popup.as_ref() {
            if let Target::Option(i) = t {
                if matches!(popup, Popup::Pick { act: Act::Picture(values), .. } if !values.get(i).is_some_and(|(_, active)| *active)) { return Out::default(); }
                let pick = matches!(popup, Popup::Pick { .. } | Popup::Add { .. });
                self.set_option(i);
                if pick {
                    return self.popup_key(K::Ok, env, now);
                }
            }
            return Out::default();
        }
        let rows = self.rows(env);
        let ids = Self::ids(&rows);
        match t {
            Target::Strip(i) => {
                self.zone = Zone::Strip;
                self.strip = i;
                self.switch_to(i, env, now)
            }
            Target::Foot => {
                self.zone = Zone::Foot;
                self.key(K::Ok, env, now)
            }
            Target::Row(i) => {
                self.zone = Zone::List;
                if !rows.get(i).is_some_and(Row::focusable) {
                    return Out::default();
                }
                self.enter(&rows, Some(i));
                self.sync_focus_id(&rows);
                match (side, rows[i].kind) {
                    (Some(d), Kind::Stepper) => {
                        let from = if matches!(rows[i].act, Act::Picture(_)) { rows[i].index.unwrap_or(if d > 0 { rows[i].options.len().saturating_sub(1) } else { 0 }) } else { self.seg };
                        if let Some(next) = rows[i].step_index(from, d) {
                            self.seg = next;
                            return self.act(&rows[i].act, Val::Index(next), env, now);
                        }
                        Out::default()
                    }
                    // the chevrons of a nudge push the value the way a key does
                    (Some(d), Kind::Nudge) => {
                        let act = rows[i].act.clone();
                        self.act(&act, Val::Delta(d), env, now)
                    }
                    _ => self.activate(&rows, i, env, now),
                }
            }
            Target::EditRow(i) => {
                if !self.edit.grab {
                    self.edit.index = i.min(rows.len());
                }
                Out::default()
            }
            Target::EditMove(i, d) => {
                self.edit.index = i;
                self.edit.grab = false;
                let j = i as i32 + d;
                if j >= 0 && (j as usize) < rows.len() {
                    let order = arrange::moved(&ids, &ids[i], d);
                    self.edit.index = j as usize;
                    return Out::effects(self.keep_order(order, true));
                }
                Out::default()
            }
            Target::EditRemove(i) => {
                self.edit.index = i;
                self.edit.grab = false;
                Out::effects(self.remove(&ids, i))
            }
            Target::EditAdd => {
                self.edit.index = rows.len();
                self.open_add(env);
                Out::default()
            }
            Target::Option(_) | Target::Pad(_) => Out::default(),
            Target::Backdrop => Out::close(),
        }
    }

    // ---- time and answers

    pub fn tick(&mut self, env: &mut Env, now: u64) -> Out {
        let mut out = Out::default();
        if self.info_answered.is_some_and(|at| now.saturating_sub(at) > 3_000) {
            self.info_answered = None;
            self.video = None;
            self.monitor.reset();
        }
        if self.info_due.is_some_and(|d| now >= d) {
            self.info_due = Some(now + 1_000);
            if env.screen_on && self.info_authority.is_some() {
                self.info_tag = self.info_tag.wrapping_add(1);
                out.effects.push(Effect::Luna { tag: self.info_tag, uri: "luna://com.webos.service.videooutput/getStatus".into(), payload: json!({}), subscribe: false });
            }
            out.effects.push(Effect::Redraw);
        }
        if self.read_due.is_some_and(|d| now >= d) {
            self.read_due = None;
            // each reading's answer rebinds rows; they are asked for once the slide is over
            out.effects.extend(env.sio.read("picture", &["pictureMode", "backlight", "energySaving", "eyeComfortMode"], None, now));
            out.effects.extend(env.sio.read("general", &["remotePointer"], None, now));
            out.effects.extend(env.sio.read("option", &["screenOff"], None, now));
            out.effects.extend(env.inputs.refresh());
            out.effects.extend(env.power.read());
            out.effects.push(Effect::Fetch { tag: tags::CAPTURE_READ, path: "/v1/capture".into() });
        }
        if self.clock_due.is_some_and(|d| now >= d) {
            self.clock_due = self.life.presented().then_some(now + 15_000);
        }
        if self.shot_due.is_some_and(|d| now >= d) {
            self.shot_due = None;
            out.effects.push(Effect::Request { tag: tags::CAPTURE_POST, method: "POST".into(), path: "/v1/capture/screenshot".into(), body: Some(json!({})) });
        }
        if let Some(Popup::Slider { shown, key_at, .. }) = self.popup.as_mut() {
            self.slider_due = Some(now + 400);
            // the card follows what the TV says when it says something else (a remote's own key, a plugin), but not while a key is moving it
            if now.saturating_sub(*key_at) >= 800 {
                let v = env.sio.num("picture.backlight", *shown);
                if v != *shown {
                    *shown = v;
                }
            }
        }
        if self.hold.tick(now) {
            self.held(env);
        }
        out
    }

    pub fn due_at(&self) -> Option<u64> {
        let slider = if matches!(self.popup, Some(Popup::Slider { .. })) { self.slider_due } else { None };
        [self.read_due, self.clock_due, self.shot_due, self.info_due, slider, self.hold.due_at(), self.life.due_at()].into_iter().flatten().min()
    }

    /// What the helper answered to the panel's requests: the capture's state after a command, the profile switch (and its PIN).
    pub fn answer(&mut self, input: &Input, env: &mut Env, now: u64) -> Out {
        if let Input::Luna { tag, payload } = input {
            if self.info_due.is_some() && *tag == self.info_tag {
                let next = info::video(payload).filter(|v| env.screen_on && self.info_authority.as_ref().is_some_and(|(context, app)| v.context == *context && v.app == *app) && self.info_authority == env.fg.video);
                if self.video.as_ref().map(|v| (&v.context, &v.app)) != next.as_ref().map(|v| (&v.context, &v.app)) {
                    self.monitor.reset();
                }
                self.video = next;
                self.info_answered = Some(now);
                return Out::effects(vec![Effect::Redraw]);
            }
        }
        let Input::Fetched { tag, status, data } = input else { return Out::default() };
        match *tag {
            t if t == tags::CAPTURE_POST => Out::effects(vec![Effect::Fetch { tag: tags::CAPTURE_READ, path: "/v1/capture".into() }]),
            t if t == tags::PROFILE_SWITCH => {
                if let Some(pin) = self.pin.as_mut() {
                    return match pin.answer(*status, data, now) {
                        Verdict::Done => {
                            self.pin = None;
                            Out::close()
                        }
                        _ => Out::default(),
                    };
                }
                match *status {
                    200..=299 => Out::close(),
                    403 if data.pointer("/error/code").and_then(Value::as_str) == Some("pin_required") => {
                        if let Some(p) = people(env).and_then(|l| l.into_iter().nth(self.strip)) {
                            self.pin = Some(Pin::new(&p.id, &p.name, tags::PROFILE_SWITCH));
                        }
                        Out::default()
                    }
                    _ => Out::default(),
                }
            }
            _ => Out::default(),
        }
    }

    // ---- what is drawn

    fn pick_detail(row: &str, i: usize, now: u64) -> String {
        if row == "sleep" && i > 0 { clock::time_of(now + u64::from(crate::host::power::Power::minutes_of(i)) * 60_000) } else { String::new() }
    }

    pub fn view(&self, env: &Env, now: u64) -> View {
        let rows = self.rows(env);
        let strip = people(env);
        let editing = self.editing;
        let popup = self.popup.as_ref().map(|p| match p {
            Popup::Slider { spec, shown, .. } => PopupView::Slider { title: spec.title.into(), text: spec.text.into(), from: spec.from, to: spec.to, step: spec.step, value: *shown, marks: spec.marks, icons: spec.icons },
            Popup::Pick { row, act, title, text, icon, options, index } => {
                let current = match act {
                    Act::Picture(values) => values.iter().position(|(value, _)| *value == env.sio.text("picture.pictureMode")),
                    _ => tiles::everything(env).iter().find(|r| r.id == *row).and_then(|r| r.index),
                };
                PopupView::Pick {
                    title: title.clone(),
                    text: (*text).into(),
                    options: options.iter().enumerate().map(|(i, o)| OptionView { label: o.clone(), icon: icon.clone(), detail: Self::pick_detail(row, i, now), current: current == Some(i), enabled: !matches!(act, Act::Picture(values) if !values.get(i).is_some_and(|(_, active)| *active)) }).collect(),
                    index: *index,
                }
            }
            Popup::Add { items, text, index } => PopupView::Pick {
                title: "Add a tile".into(),
                text: (*text).into(),
                options: items.iter().map(|(_, icon, label)| OptionView { label: label.clone(), icon: icon.clone(), detail: String::new(), current: false, enabled: true }).collect(),
                index: *index,
            },
        });
        let focus = if self.popup.is_some() {
            match &self.popup {
                Some(Popup::Pick { index, .. }) | Some(Popup::Add { index, .. }) => Some(Focus::Option(*index)),
                _ => None,
            }
        } else if let Some(pin) = &self.pin {
            Some(Focus::Pad(pin.focus()))
        } else if editing {
            Some(if self.edit.index >= rows.len() { Focus::EditAdd } else { Focus::EditRow(self.edit.index) })
        } else {
            match self.zone {
                Zone::Foot => Some(Focus::Foot),
                Zone::Strip => Some(Focus::Strip(self.strip)),
                Zone::List => self.stop.map(Focus::Row),
            }
        };
        let (clock_text, head) = if editing {
            let hint = if self.edit.grab {
                vec![Hint { keys: vec![(Key::Up, false), (Key::Down, false)], action: Some(Action::Move), label: None }, Hint::key(Key::Ok, Action::Confirm)]
            } else {
                vec![Hint::key(Key::Ok, Action::Move), Hint::key(Key::Back, Action::Confirm)]
            };
            (String::new(), hint)
        } else if self.pin.is_some() {
            // the PIN pad: the clock gives way to what the keys do (OK types the digit under the focus, Back gives the list back)
            (String::new(), vec![Hint::key(Key::Ok, Action::Confirm), Hint::key(Key::Back, Action::Close)])
        } else {
            (clock::time_of(now), Vec::new())
        };
        View {
            up: self.life.presented(),
            editing,
            title: if editing { "Edit tiles".into() } else { "Quick settings".into() },
            clock: clock_text,
            info: self.monitor.snapshot(),
            video_info: self.video.as_ref().filter(|v| env.screen_on && self.info_due.is_some() && self.info_authority.as_ref().is_some_and(|(context, app)| v.context == *context && v.app == *app) && self.info_authority == env.fg.video && self.info_answered.is_some_and(|at| now.saturating_sub(at) <= 3_000)).map(|v| VideoInfo { width: v.width, height: v.height, fps: v.fps }),
            hints_head: head,
            hints_foot: if env.popups_on() { vec![Hint::hold(Key::Ok, Action::Edit), Hint::key(Key::Back, Action::Close)] } else { Vec::new() },
            strip: if editing || self.pin.is_some() { None } else { strip.clone() },
            pad: self.pin.as_ref().map(|p| PadView { avatar: strip.as_ref().and_then(|l| l.iter().find(|x| x.id == p.id).cloned()), pin: p.view(now) }),
            rows: if editing { Vec::new() } else { rows.clone() },
            edit: editing.then_some(EditView { rows, index: self.edit.index, grab: self.edit.grab, on_remove: self.edit.on_remove }),
            popup,
            focus,
            popups_on: env.popups_on(),
            over_video: !env.fg.home,
        }
    }

    pub fn is_editing(&self) -> bool {
        self.editing
    }

    pub fn zone(&self) -> Zone {
        self.zone
    }

    pub fn stop(&self) -> Option<usize> {
        self.stop
    }

    pub fn popup_open(&self) -> bool {
        self.popup.is_some()
    }

    pub fn pin(&self) -> Option<&Pin> {
        self.pin.as_ref()
    }
}

#[cfg(test)]
pub(crate) mod tests;
