//! Pure navigation and application rules. Session decisions remain in lumagame-core; the shell only performs commands.
use lumagame_core::{
    activity::{self, LastPlayed, Place},
    catalog::{self, ArtKind, Game, Listing, SaveCompat},
    engine::{Codec, Descriptor, EngineKind, Feature, Manifest, Presentation},
    history::Played,
    ids::{EngineId, GameId, SessionId, SourceKey, TargetId},
    ipc::{self, AuthState, FromEngine, Readiness, Request, SessionState, Stats, StreamInfo, Target},
    session::{self, Phase, Session},
    settings::{FieldKind, SettingsSchema, Value, Values},
    state::{OpenSession, State},
};
use lumatv_gfn_pad::{
    Curve, MAX_PADS, RawReport, Shaping, button,
    lab::{Drift, Summary, Timing},
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use super::helper::{ControllerChange, GameSir, GameSirState, Leadjoy, LeadjoyState, LeadjoyView};
use super::stats::{Mode as StatsMode, Overlay, Telemetry};
use super::diagnostics::{Config as DiagnosticsConfig, Level as DiagnosticsLevel, Status as DiagnosticsStatus};
use super::input::{Labels, PadKind, Source, Sources};
use super::uplink::{self, Uplink};
pub mod accounts;
pub mod facts;
pub mod game;
pub mod home;
pub mod launch;
pub mod library;
pub mod nav;
pub mod quick;
mod refresh;
pub mod settings;
pub mod text;
#[cfg(test)]
pub(crate) mod tests;
use accounts::{LinkFlow, PC_SEARCH_MS};
use game::{HubTab, media::Media};
use home::{HomeItem, HomeRow};
use launch::{Cause, IdleWarning, QueueWatch};
use library::{Filter, Sort};
use nav::{Neighbours, Owner};
use settings::{Lab, PadMode, Trip};
pub use refresh::Line as RefreshLine;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Ok,
    Back,
    Menu,
    /// The pad's X button: the menus' second action, which the remote has no key for.
    Alt,
    Guide,
}
impl Key {
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            103 => Self::Up,
            108 => Self::Down,
            105 => Self::Left,
            106 => Self::Right,
            28 | 352 | 353 => Self::Ok,
            412 | 158 | 1 | 14 => Self::Back,
            139 => Self::Menu,
            172 | 316 => Self::Guide,
            _ => return None,
        })
    }
    pub fn from_pad(key: ipc::PadKey) -> Option<Self> {
        Some(match key {
            ipc::PadKey::Up => Self::Up,
            ipc::PadKey::Down => Self::Down,
            ipc::PadKey::Left => Self::Left,
            ipc::PadKey::Right => Self::Right,
            ipc::PadKey::Ok => Self::Ok,
            ipc::PadKey::Back => Self::Back,
            ipc::PadKey::Menu => Self::Menu,
            ipc::PadKey::Alt => Self::Alt,
            _ => return None,
        })
    }
}
#[derive(Clone, Debug)]
pub enum Input {
    Key {
        key: Key,
        down: bool,
    },
    /// A press on the controller in `slot`, as the pad reader makes it (the remote's keys are [`Input::Key`]).
    PadKey {
        slot: u8,
        key: Key,
        down: bool,
    },
    /// The connected controllers' names by slot, whenever they change.
    Controllers([Option<String>; MAX_PADS]),
    Engine {
        engine: EngineId,
        message: FromEngine,
    },
    EngineGone(EngineId),
    Foreground(bool),
    Tick {
        now_ms: u64,
        unix_s: u64,
    },
    Close,
    Overlay,
    Notice(String),
    ArtReady {
        url: String,
        path: PathBuf,
    },
    ArtFailed {
        url: String,
    },
    Details {
        game: GameId,
        details: Box<GameDetails>,
    },
    Text(String),
    /// A click supplies its hit-test destination; pointer movement never changes focus.
    Click {
        focus: Focus,
        down: bool,
    },
    /// The pads as the controller lab reads them, on every pass of the shell's loop while [`Model::pad_mode`] is not the menu's: the
    /// reports since the last pass, the connected pads' names and the number of the frame the shell draws next.
    Lab {
        now_ns: u64,
        reports: Vec<(u8, RawReport)>,
        names: [Option<String>; MAX_PADS],
        frame: u64,
    },
    GameSir(Result<GameSirState, String>),
    Leadjoy(Result<LeadjoyState, String>),
    Frames(crate::lifecycle::Frames),
    Display(serde_json::Value),
    StatsSetting(Overlay),
    DiagnosticsStatus(DiagnosticsStatus),
    /// The TV's uplink as the shell read it; absent when the TV has no default route.
    Uplink(Option<Uplink>),
}
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    // Core: engines, saved state and the shell.
    Send { engine: EngineId, request: Request },
    Enable { engine: EngineId, on: bool },
    KillEngine(EngineId),
    SaveState,
    SaveCatalog,
    FetchArt { url: String },
    FetchDetails { game: GameId },
    Exit,
    // game/media
    PlayMedia { url: String },
    // settings
    OpenControllerSettings,
    Rumble { slot: u8, low: u16, high: u16 },
    GameSir(Option<ControllerChange>),
    Leadjoy { view: LeadjoyView, change: Option<ControllerChange> },
    Diagnostics(DiagnosticsConfig),
    DiagnosticTest,
    // launch
    Checkpoint(OpenSession),
    ClearCheckpoint,
    LowLatency(bool),
    /// The in-stream menu's Steam overlay, for the shell to hand the engine and to note in diagnostics; `None` when no session is open.
    SteamOverlay(Option<(EngineId, SessionId)>),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Density {
    Compact,
    #[default]
    Standard,
    Cinematic,
}
impl Density {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Standard, Self::Cinematic];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|v| *v == self).unwrap_or(1)
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Standard => "Standard",
            Self::Cinematic => "Cinematic",
        }
    }
    pub fn cover_width(self) -> f32 {
        [224.0, 280.0, 336.0][self.index()]
    }
    pub fn columns(self) -> usize {
        [7, 6, 5][self.index()]
    }
    pub fn hero_height(self) -> f32 {
        [240.0, 360.0, 480.0][self.index()]
    }
    pub fn logo_height(self) -> f32 {
        [72.0, 120.0, 176.0][self.index()]
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub family: String,
    pub light: bool,
    pub hero: Density,
    pub covers: Density,
    pub text_percent: u16,
    /// Whose markings hints draw on a face button; automatic follows the pad in use.
    pub button_labels: Labels,
    pub reduce_motion: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub hdr: bool,
    pub codec: Option<Codec>,
    /// Smooth hands a stream's frames to the decoder evenly; off hands each over as it arrives.
    pub smooth_pacing: bool,
    pub bitrate_mbps: u32,
    pub stats: Overlay,
    /// The stick settings every engine that drives a pad applies: dead zone, anti-deadzone (both percent) and response curve.
    pub deadzone: u8,
    pub anti_deadzone: u8,
    pub stick_curve: Curve,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            family: "rose-pine".into(),
            light: false,
            hero: Density::Standard,
            covers: Density::Standard,
            text_percent: 100,
            button_labels: Labels::Automatic,
            reduce_motion: false,
            width: 3840,
            height: 2160,
            fps: 60,
            hdr: false,
            codec: None,
            smooth_pacing: true,
            bitrate_mbps: 80,
            stats: Overlay::default(),
            deadzone: 8,
            anti_deadzone: 0,
            stick_curve: Curve::Linear,
        }
    }
}
impl Preferences {
    fn read(v: &serde_json::Value) -> Self {
        let mut p = Self::default();
        if let Some(s) = v["family"].as_str() {
            p.family = s.into();
        }
        p.light = v["light"].as_bool().unwrap_or(p.light);
        p.hero = Density::ALL[v["hero"].as_u64().unwrap_or(1).min(2) as usize];
        p.covers = Density::ALL[v["covers"].as_u64().unwrap_or(1).min(2) as usize];
        p.text_percent = match v["text_percent"].as_u64() {
            Some(150) => 150,
            Some(200) => 200,
            _ => 100,
        };
        p.button_labels = Labels::from_key(v["button_labels"].as_str().unwrap_or_default());
        p.reduce_motion = v["reduce_motion"].as_bool().unwrap_or(false);
        let resolution = v["height"].as_u64().unwrap_or(2160);
        (p.width, p.height) = match resolution {
            720 => (1280, 720),
            1080 => (1920, 1080),
            1440 => (2560, 1440),
            _ => (3840, 2160),
        };
        p.fps = if v["fps"].as_u64() == Some(120) { 120 } else { 60 };
        p.hdr = v["hdr"].as_bool().unwrap_or(false);
        p.codec = match v["codec"].as_str() {
            Some("h264") => Some(Codec::H264),
            Some("h265") => Some(Codec::H265),
            Some("av1") if p.fps <= 60 => Some(Codec::Av1),
            _ => None,
        };
        p.smooth_pacing = v["frame_pacing"].as_str() != Some("latency");
        p.bitrate_mbps = v["bitrate_mbps"].as_u64().unwrap_or(80).clamp(u64::from(uplink::BITRATE_MIN_MBPS), u64::from(uplink::BITRATE_MAX_MBPS)) as u32;
        p.stats = Overlay::read(&v["stats"]);
        p.deadzone = v["deadzone"].as_u64().map_or(p.deadzone, |n| n.min(u64::from(DEADZONE_MAX)) as u8);
        p.anti_deadzone = v["anti_deadzone"].as_u64().map_or(p.anti_deadzone, |n| n.min(u64::from(ANTI_DEADZONE_MAX)) as u8);
        p.stick_curve = v["stick_curve"].as_str().and_then(Curve::from_name).unwrap_or_default();
        p
    }
    fn json(&self) -> serde_json::Value {
        serde_json::json!({"family":self.family,"light":self.light,"hero":self.hero.index(),"covers":self.covers.index(),"text_percent":self.text_percent,"button_labels":self.button_labels.key(),"reduce_motion":self.reduce_motion,"width":self.width,"height":self.height,"fps":self.fps,"hdr":self.hdr,"codec":self.codec.map_or("auto",Codec::as_str),"frame_pacing":self.frame_pacing(),"bitrate_mbps":self.bitrate_mbps,"stats":self.stats,"deadzone":self.deadzone,"anti_deadzone":self.anti_deadzone,"stick_curve":self.stick_curve.name()})
    }
    fn frame_pacing(&self) -> &'static str {
        if self.smooth_pacing { "smooth" } else { "latency" }
    }
    pub fn text_scale(&self) -> f32 {
        f32::from(self.text_percent) / 100.0
    }
    pub fn shaping(&self) -> Shaping {
        Shaping { deadzone_percent: self.deadzone, anti_deadzone_percent: self.anti_deadzone, curve: self.stick_curve }
    }
}
/// The engines' schemas take these as the stick settings' upper bounds.
pub const DEADZONE_MAX: u8 = 20;
pub const ANTI_DEADZONE_MAX: u8 = 30;
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameDetails {
    pub description: String,
    pub year: Option<u32>,
    pub genres: Vec<String>,
    pub players: Option<String>,
    pub developer: Option<String>,
    pub capabilities: Vec<String>,
    pub hub: lumagame_meta::Hub,
    pub cloud_saves: Option<bool>,
    pub media: Vec<Media>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    // home
    #[default]
    Home,
    // library
    Library,
    Search,
    // game
    Hub,
    // game/media
    Media,
    // settings
    Settings,
    Lab,
    GameSir,
    Leadjoy,
    // accounts
    Accounts,
    // launch
    Launch,
    Running,
    Overlay,
}
/// What opens over the page. F1 turns `View::sheet` into a stack of these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    // game
    Chooser,
    // library
    Sort,
    Genre,
    // settings
    Theme,
    /// Confirms a controller setting that removes something, such as a macro.
    Delete,
    // accounts
    Auth,
    // launch
    EndSession,
    // text
    AddHost,
    EditText,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Focus {
    #[default]
    None,
    // core
    Nav(usize),
    // home
    Hero(usize),
    Home(usize, usize),
    FirstRun(usize),
    // library
    Filter(usize),
    Grid(usize),
    SearchField,
    // text
    Keyboard(usize),
    // game
    HubTop(usize),
    HubAction(usize),
    Tab(usize),
    Way(usize),
    /// A row of the achievements or news section.
    Item(usize),
    Media(usize),
    Remember,
    // settings (`Setting` is also a row of an integration's page)
    Rail(usize),
    Setting(usize),
    Theme(usize),
    /// The controller lab's actions: test buttons, rumble left, rumble right, flash test, measure the stream.
    Lab(usize),
    // accounts
    Integration(usize),
    Target(usize),
    EngineField(usize),
    // any layer's rows and buttons
    SheetRow(usize),
    SheetAction(usize),
    // launch
    Overlay(usize),
    /// The launch screen's Cancel: a key reaches it, so an impatient second OK never cancels the wait.
    Cancel,
}
#[derive(Clone, Debug)]
pub struct Integration {
    pub manifest: Manifest,
    pub descriptor: Option<Descriptor>,
    pub schema: SettingsSchema,
    pub targets: Vec<Target>,
    pub error: Option<(ipc::ErrorCode, String)>,
}
impl Integration {
    /// An account integration with an account linked has a "Sign out" row on its page.
    pub fn can_sign_out(&self) -> bool {
        self.manifest.kind == EngineKind::Account && !self.targets.is_empty()
    }
}
#[derive(Clone, Debug)]
pub struct View {
    pub page: Page,
    pub sheet: Option<Layer>,
    pub focus: Focus,
    pub place: settings::Place,
    pub hub_tab: usize,
    pub selected: Option<GameId>,
    pub featured: Option<GameId>,
    pub home_row: usize,
    pub first_grid_row: usize,
    pub query: String,
    pub sort: Sort,
    pub filter: Filter,
    pub chooser_selected: Option<SourceKey>,
    pub remember: bool,
    pub selected_engine: Option<EngineId>,
    pub notice: Option<String>,
    pub auth: Option<LinkFlow>,
    pub stream: Option<StreamInfo>,
    pub stats: Stats,
    pub media: usize,
    pub address: String,
    pub edit_field: Option<usize>,
    pub keyboard_lower: bool,
    pub idle: Option<IdleWarning>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            page: Page::Home,
            sheet: None,
            focus: Focus::Hero(0),
            place: settings::Place::Appearance,
            hub_tab: 0,
            selected: None,
            featured: None,
            home_row: 0,
            first_grid_row: 0,
            query: String::new(),
            sort: Sort::Title,
            filter: Filter::default(),
            chooser_selected: None,
            remember: false,
            selected_engine: None,
            notice: None,
            auth: None,
            stream: None,
            stats: Stats::default(),
            media: 0,
            address: String::new(),
            edit_field: None,
            keyboard_lower: false,
            idle: None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Model {
    state: State,
    listing: Listing,
    games: Vec<Game>,
    pub integrations: Vec<Integration>,
    pub prefs: Preferences,
    pub uplink: Option<Uplink>,
    pub view: View,
    pub details: BTreeMap<GameId, GameDetails>,
    pub art: BTreeMap<String, PathBuf>,
    pub home_rows: Vec<HomeRow>,
    pub filtered: Vec<GameId>,
    /// Every genre by its one label, with the games under it: the metadata provider's and every service's, joined (see `activity`).
    genre_games: BTreeMap<String, BTreeSet<GameId>>,
    pub navigation: Vec<Neighbours>,
    pub lab: Lab,
    pub gamesir: GameSir,
    pub leadjoy: Leadjoy,
    pub telemetry: Telemetry,
    pub diagnostics: DiagnosticsConfig,
    pub diagnostics_status: DiagnosticsStatus,
    session: Option<Session>,
    session_start: u64,
    queue: QueueWatch,
    foreground: bool,
    now_ms: u64,
    unix_s: u64,
    revision: u64,
    next_session: u64,
    pressed: Option<(Key, Focus)>,
    back: Vec<(Page, Focus)>,
    sheet_return: Focus,
    closing: bool,
    ended_by: Option<Cause>,
    requested_art: BTreeSet<String>,
    requested_details: BTreeSet<GameId>,
    owned: BTreeMap<GameId, Vec<ipc::Unlocked>>,
    requested_owned: BTreeSet<GameId>,
    resting: Option<(GameId, u64)>,
    pages: BTreeMap<(EngineId, TargetId), Vec<ipc::LibraryEntry>>,
    sources: Sources,
    refresher: refresh::Refresher,
}
impl Model {
    pub fn new(state: State, mut listing: Listing, manifests: Vec<Manifest>) -> Self {
        listing.unchecked();
        let prefs = Preferences::read(&state.profile().ui);
        let mut m = Self {
            state,
            listing,
            games: Vec::new(),
            integrations: {
                // Settings lists integrations under one heading per kind, so each kind has to be contiguous.
                let mut list: Vec<_> = manifests
                    .into_iter()
                    .map(|manifest| Integration { manifest, descriptor: None, schema: SettingsSchema::default(), targets: Vec::new(), error: None })
                    .collect();
                list.sort_by_key(|e| match e.manifest.kind {
                    EngineKind::Cloud => 0,
                    EngineKind::Host => 1,
                    EngineKind::Account => 2,
                    _ => 3,
                });
                list
            },
            prefs,
            uplink: None,
            view: View::default(),
            details: BTreeMap::new(),
            art: BTreeMap::new(),
            home_rows: Vec::new(),
            filtered: Vec::new(),
            genre_games: BTreeMap::new(),
            navigation: Vec::new(),
            lab: Lab::default(),
            gamesir: GameSir::default(),
            leadjoy: Leadjoy::default(),
            telemetry: Telemetry::default(),
            diagnostics: DiagnosticsConfig::default(),
            diagnostics_status: DiagnosticsStatus::Off,
            session: None,
            session_start: 0,
            queue: QueueWatch::default(),
            foreground: true,
            now_ms: 0,
            unix_s: 0,
            revision: 1,
            next_session: 1,
            pressed: None,
            back: Vec::new(),
            sheet_return: Focus::None,
            closing: false,
            ended_by: None,
            requested_art: BTreeSet::new(),
            requested_details: BTreeSet::new(),
            owned: BTreeMap::new(),
            requested_owned: BTreeSet::new(),
            resting: None,
            pages: BTreeMap::new(),
            sources: Sources::default(),
            refresher: refresh::Refresher::default(),
        };
        m.rebuild_catalog();
        m.rebuild_navigation();
        m
    }
    pub fn state(&self) -> &State {
        &self.state
    }
    pub fn listing(&self) -> &Listing {
        &self.listing
    }
    pub fn games(&self) -> &[Game] {
        &self.games
    }
    /// The kind of input whose glyphs hints draw.
    pub fn input_source(&self) -> Source {
        self.sources.glyphs()
    }
    /// Whose markings hints draw on a face button: the Controllers setting's, or the pad in use.
    pub fn pad_kind(&self) -> PadKind {
        self.prefs.button_labels.kind(self.sources.pad())
    }
    pub fn foreground(&self) -> bool {
        self.foreground
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }
    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }
    pub fn session_active(&self) -> bool {
        self.session.as_ref().is_some_and(|s| !s.is_done())
    }
    /// The session as diagnostics sees it: how it stands after a step, with what was asked for beside what the stream turned out to be.
    pub fn observed(&self) -> Option<super::observer::Seen<'_>> {
        let s = self.session.as_ref()?;
        Some(super::observer::Seen {
            id: s.id.0,
            engine: super::observer::engine_word(s.source.engine.as_str()),
            phase: s.phase(),
            stream: self.view.stream.as_ref(),
            ended_by: self.ended_by.map(Cause::word),
            played_s: s.played(self.now_ms),
            want: super::observer::Want { hdr: self.prefs.hdr, fps: self.prefs.fps, height: self.prefs.height },
        })
    }
    pub fn streaming(&self) -> bool {
        self.session.as_ref().is_some_and(|s| {
            !s.is_done()
                && self.integration(&s.source.engine).and_then(|e| e.descriptor.as_ref()).is_none_or(|d| d.capabilities.presentation != Presentation::App)
        })
    }
    pub fn wants_pads(&self) -> bool {
        self.foreground && !self.session_active()
    }
    pub fn wants_frames(&self) -> bool {
        self.foreground && (self.view.page != Page::Running || self.hud_visible() || self.idle_left().is_some())
    }
    pub fn game(&self, id: GameId) -> Option<&Game> {
        self.games.iter().find(|g| g.id == id)
    }
    pub fn selected_game(&self) -> Option<&Game> {
        self.view.selected.and_then(|id| self.game(id))
    }
    pub fn integration(&self, id: &EngineId) -> Option<&Integration> {
        self.integrations.iter().find(|e| &e.manifest.id == id)
    }
    pub fn enabled(&self, id: &EngineId) -> bool {
        self.state.engines.get(id).is_some_and(|e| e.enabled)
    }
    pub fn target(&self, key: &SourceKey) -> Option<&Target> {
        self.integration(&key.engine)?.targets.iter().find(|t| t.id == key.target)
    }
    pub fn engine_name<'a>(&'a self, id: &'a EngineId) -> &'a str {
        self.integration(id).map_or(id.as_str(), |e| e.manifest.name.as_str())
    }
    /// The latest play of the game on this TV or on any service, and where it was (see `activity`).
    pub fn last_played(&self, id: GameId) -> Option<LastPlayed> {
        activity::last_played(self.game(id)?, &self.state.profile().history)
    }
    /// How long ago that was. A service that gives a date and no time is told by the day.
    pub fn since_played(&self, id: GameId) -> Option<String> {
        let last = self.last_played(id)?;
        let seconds = self.unix_s.saturating_sub(last.at);
        let days = seconds / 86400;
        Some(if last.day_only {
            match days {
                0 => "Today".into(),
                1 => "Yesterday".into(),
                _ => format!("{days} days ago"),
            }
        } else if seconds < 60 {
            "Just now".into()
        } else if seconds < 3600 {
            format!("{} min ago", seconds / 60)
        } else if seconds < 86400 {
            format!("{} h ago", seconds / 3600)
        } else {
            format!("{days} days ago")
        })
    }
    /// A place as a game's page names it: "this TV", or the service's own name.
    pub fn place_name(&self, place: &Place) -> String {
        match place {
            Place::ThisTv => "this TV".into(),
            Place::Service(id) => self.engine_name(id).to_owned(),
        }
    }
    /// The longest play time of the game in any one place, in seconds, and which place. Never a sum (see `activity`).
    pub fn play_time_of(&self, id: GameId) -> Option<(u64, Place)> {
        activity::longest_play(self.game(id)?, &self.state.profile().history)
    }
    /// The game's genres: the metadata provider's and every service's, one label each.
    pub fn game_genres(&self, id: GameId) -> Vec<String> {
        let Some(game) = self.game(id) else { return Vec::new() };
        activity::genres(game, self.details.get(&id).map_or(&[][..], |d| d.genres.as_slice()))
    }
    /// The finer tags services give the game, without its genres.
    pub fn game_tags(&self, id: GameId) -> Vec<String> {
        self.game(id).map(activity::tags).unwrap_or_default()
    }
    /// The games listed under one genre, in title order.
    pub fn in_genre(&self, name: &str) -> Vec<GameId> {
        let Some(ids) = self.genre_games.get(name) else { return Vec::new() };
        self.games.iter().filter(|g| ids.contains(&g.id)).map(|g| g.id).collect()
    }
    /// The names of the services that mark the game a favourite.
    pub fn favourite_on(&self, id: GameId) -> Vec<&str> {
        self.game(id).map(|game| activity::favourite_on(game).into_iter().map(|engine| self.engine_name(engine)).collect()).unwrap_or_default()
    }
    /// Whether the owner or a service marks the game a favourite.
    pub fn is_favourite(&self, id: GameId) -> bool {
        self.game(id).is_some_and(|game| activity::is_favourite(game, &self.state.profile().user.favourites))
    }
    /// Every genre any game has, once each, in alphabetical order.
    pub fn genres(&self) -> Vec<String> {
        self.genre_games.keys().cloned().collect()
    }
    pub fn art_url(&self, game: GameId, kind: ArtKind) -> Option<String> {
        self.game(game)?.art(kind).into_iter().next()
    }
    pub fn art_path(&self, url: &str) -> Option<PathBuf> {
        self.art.get(url).cloned()
    }
    pub fn step(&mut self, input: Input) -> Vec<Command> {
        let mut out = Vec::new();
        let mut changed = true;
        match input {
            Input::Key { key, down } => {
                let switched = down && self.sources.used(Source::Remote);
                changed = self.key(key, down, &mut out) | switched;
            }
            Input::PadKey { slot, key, down } => {
                let switched = down && self.sources.used_pad(usize::from(slot));
                changed = self.key(key, down, &mut out) | switched;
            }
            Input::Controllers(names) => changed = self.sources.connected(&names),
            Input::Click { focus, down } => {
                if self.navigation.iter().any(|n| n.id == focus) {
                    let switched = down && self.sources.used(Source::Remote);
                    self.view.focus = focus;
                    changed = self.key(Key::Ok, down, &mut out) | switched;
                }
            }
            Input::Engine { engine, message } => {
                changed = self.view.page != Page::Running || !matches!(message.event, ipc::Event::Stats { .. });
                self.engine_event(engine, message.event, &mut out);
            }
            Input::EngineGone(id) => {
                self.pages.retain(|(engine, _), _| *engine != id);
                self.refresh_gone(&id);
                if let Some(e) = self.integrations.iter_mut().find(|e| e.manifest.id == id) {
                    e.descriptor = None;
                    e.error = Some((ipc::ErrorCode::Network, "Connection lost".into()));
                    for target in &mut e.targets {
                        target.reach = ipc::Reach::Offline;
                    }
                }
                if self.session.as_ref().is_some_and(|s| s.source.engine == id) {
                    self.session_input(session::Input::EngineGone, &mut out);
                }
            }
            Input::Foreground(on) => {
                self.foreground = on;
                self.refresh_foreground(on, &mut out);
                if on {
                    self.requested_owned.clear();
                }
                self.pressed = None;
                self.stats_request(&mut out);
                if !on {
                    self.lab.capture = None;
                    self.lab.flash = None;
                    self.session_input(session::Input::Background, &mut out);
                }
            }
            Input::Close => {
                if self.session_active() {
                    self.closing = true;
                    self.session_input(session::Input::Stop { end_game: false }, &mut out);
                } else {
                    out.push(Command::Exit);
                }
            }
            Input::Overlay => self.open_overlay(&mut out),
            Input::Tick { now_ms, unix_s } => {
                let second = self.now_ms / 1000 != now_ms / 1000;
                self.now_ms = now_ms;
                self.unix_s = unix_s;
                self.session_input(session::Input::Tick, &mut out);
                changed = second && (self.view.page == Page::Overlay || self.view.idle.is_some() || self.view.auth.as_ref().is_some_and(|a| a.expires_ms.is_some()));
                changed |= self.refresh_tick(second);
                self.look_ahead(&mut out);
                if self.hud_visible() {
                    let asked = self.asked();
                    changed |= self.telemetry.tick(now_ms, &asked);
                    changed |= self.telemetry.rest.tick(now_ms);
                }
                // QR engines check once more at the deadline: the phone may already have approved.
                if self.view.auth.as_ref().is_some_and(|a| a.expires_ms.is_some_and(|end| now_ms >= end) && matches!(a.state, Some(AuthState::DeviceCode { .. }))) {
                    if let Some(auth) = &mut self.view.auth {
                        auth.state = Some(AuthState::Failed { reason: ipc::AuthFail::Expired, message: "The code expired. Start sign-in again.".into() });
                    }
                    changed = true;
                }
                let host = self.view.auth.as_ref().is_some_and(|a| self.integration(&a.engine).is_some_and(|e| e.manifest.kind == EngineKind::Host));
                if let Some(a) = self.view.auth.as_mut().filter(|a| {
                    host && !a.no_pcs && a.state.is_none() && a.target.is_none() && now_ms >= a.since_ms + PC_SEARCH_MS
                }) {
                    a.no_pcs = true;
                    changed = true;
                }
            }
            Input::Notice(s) => self.view.notice = Some(s),
            Input::ArtReady { url, path } => {
                self.art.insert(url, path);
            }
            Input::ArtFailed { url } => {
                self.requested_art.insert(url);
            }
            Input::Details { game, details } => {
                self.details.insert(game, *details);
                self.rebuild_rows();
                self.rebuild_filter();
            }
            Input::Text(text) => match self.view.sheet {
                Some(Layer::Auth) => {
                    if let Some(a) = &mut self.view.auth {
                        let max = match a.state {
                            Some(AuthState::EnterPin { digits }) => usize::from(digits),
                            _ => 12,
                        };
                        a.pin.extend(text.chars().filter(char::is_ascii_digit).take(max.saturating_sub(a.pin.len())));
                    }
                }
                Some(Layer::AddHost | Layer::EditText) => {
                    self.view.address.extend(text.chars().filter(|c| !c.is_control()).take(255usize.saturating_sub(self.view.address.chars().count())))
                }
                _ => {
                    self.view.query.extend(text.chars().filter(|c| !c.is_control()).take(128usize.saturating_sub(self.view.query.chars().count())));
                    self.rebuild_filter();
                }
            },
            Input::Lab { now_ns, reports, names, frame } => {
                let pressed = reports.iter().find(|(_, report)| report.buttons != 0).map(|(slot, _)| usize::from(*slot));
                let switched = pressed.is_some_and(|slot| self.sources.used_pad(slot));
                changed = self.lab_input(now_ns, reports, names, frame) | switched;
            }
            Input::Frames(frames) => { self.telemetry.frames(frames, self.now_ms); changed = false; }
            Input::Display(display) => { self.telemetry.display(display, self.now_ms); changed = false; }
            Input::StatsSetting(setting) => {
                changed = setting.changed >= self.prefs.stats.changed && setting != self.prefs.stats;
                if changed {
                    self.prefs.stats = setting;
                    if !setting.mode.on() { self.telemetry = Telemetry::default(); }
                    self.save_prefs(&mut out);
                    self.stats_request(&mut out);
                }
            }
            Input::DiagnosticsStatus(status) => { changed = self.diagnostics_status != status; self.diagnostics_status = status; }
            Input::Uplink(uplink) => {
                changed = self.uplink != uplink;
                if changed {
                    self.uplink = uplink;
                    self.follow_uplink(&mut out);
                }
            }
            Input::GameSir(result) => {
                self.gamesir.pending = false;
                match result {
                    Ok(state) => { self.gamesir.state = state; self.gamesir.error = None; }
                    Err(error) => {
                        self.gamesir.error = Some(error);
                        if self.view.page == Page::GameSir && self.view.sheet.is_none() { self.view.focus = Focus::Setting(0); }
                    }
                }
            }
            Input::Leadjoy(result) => {
                self.leadjoy.pending = false;
                match result {
                    Ok(state) => {
                        if let LeadjoyState::Ready(profile) = &state {
                            if self.view.page == Page::Leadjoy && self.view.sheet.is_none() {
                                if let (Focus::Setting(row @ 1..), LeadjoyState::Ready(previous)) = (self.view.focus, &self.leadjoy.state) {
                                    let key = previous.schema.fields().nth(row - 1).map(|field| field.key.as_str());
                                    let candidates = || profile.schema.fields().enumerate().filter(|(_, field)| !matches!(field.kind, FieldKind::Note));
                                    let chosen = candidates().find(|(_, field)| Some(field.key.as_str()) == key)
                                        .or_else(|| candidates().min_by_key(|(index, _)| (*index + 1).abs_diff(row)));
                                    self.view.focus = chosen.map_or(Focus::Setting(0), |(index, _)| Focus::Setting(index + 1));
                                }
                            }
                            self.leadjoy.view = profile.view.clone();
                        }
                        self.leadjoy.state = state; self.leadjoy.error = None;
                    }
                    Err(error) => {
                        self.leadjoy.error = Some(error);
                        if self.view.page == Page::Leadjoy && self.view.sheet.is_none() { self.view.focus = Focus::Setting(0); }
                    }
                }
            }
        }
        // Running and the in-stream menu exist only while the stream runs: Running draws nothing over the video plane, so either page
        // outliving its session is a black screen that no key, and no relaunch, leaves.
        if matches!(self.view.page, Page::Running | Page::Overlay) && !self.session.as_ref().is_some_and(|s| *s.phase() == Phase::Running) {
            self.view.sheet = None;
            self.view.hub_tab = 0;
            (self.view.page, self.view.focus) = if self.view.selected.is_some() { (Page::Hub, Focus::HubAction(0)) } else { (Page::Home, Focus::Nav(0)) };
            changed = true;
        }
        if changed || !out.is_empty() {
            self.revision = self.revision.wrapping_add(1);
            self.rebuild_navigation();
            self.request_visible_art(&mut out);
        }
        out
    }
    fn send(out: &mut Vec<Command>, engine: &EngineId, request: Request) {
        out.push(Command::Send { engine: engine.clone(), request });
    }
    fn rebuild_catalog(&mut self) {
        let enabled = self.state.enabled_engines();
        let (games, merges) = catalog::build(&mut self.state.identity, self.listing.enabled(&enabled));
        self.state.apply_merges(&merges);
        self.games = games;
        for game in &mut self.games {
            game.sources.sort_by_key(|source| self.integrations.iter().position(|e| e.manifest.id == source.engine).unwrap_or(usize::MAX));
        }
        if self.view.featured.is_none_or(|id| self.game(id).is_none()) {
            self.view.featured = activity::recently_played(&self.games, &self.state.profile().history).first().copied().or_else(|| self.games.first().map(|g| g.id));
        }
        self.rebuild_rows();
        self.rebuild_filter();
    }
    /// Groups the games by genre from every source of genres, one label each.
    fn rebuild_genres(&mut self) {
        let mut by_genre: BTreeMap<String, BTreeSet<GameId>> = BTreeMap::new();
        for game in &self.games {
            for genre in activity::genres(game, self.details.get(&game.id).map_or(&[][..], |d| d.genres.as_slice())) {
                by_genre.entry(genre).or_default().insert(game.id);
            }
        }
        self.genre_games = by_genre;
    }
    fn show_page(&mut self, page: Page, focus: Focus) {
        if page != self.view.page {
            self.back.push((self.view.page, self.view.focus));
            // A notice belongs to the page that raised it.
            self.view.notice = None;
        }
        self.view.page = page;
        self.view.sheet = None;
        self.view.focus = focus;
        self.pressed = None;
    }
    /// Whether `id`'s error is about what the screen shows. Elsewhere it stays on the integration's own card (`error`), so a background
    /// failure (This TV's app listing at start-up) does not turn up on an unrelated page such as the controller lab.
    fn engine_in_view(&self, id: &EngineId) -> bool {
        match (self.view.sheet, self.view.page) {
            (Some(_), _) => self.view.auth.as_ref().map(|a| &a.engine).or(self.view.selected_engine.as_ref()) == Some(id),
            (None, Page::Hub) => self.view.selected.and_then(|g| self.play_target(g)).is_some_and(|k| k.engine == *id),
            _ => false,
        }
    }
    pub fn top_layer(&self) -> Option<&Layer> {
        self.view.sheet.as_ref()
    }
    fn open_sheet(&mut self, sheet: Layer, focus: Focus) {
        self.sheet_return = self.view.focus;
        self.view.sheet = Some(sheet);
        self.view.focus = focus;
        self.pressed = None;
    }
    fn close_sheet(&mut self) {
        self.view.sheet = None;
        self.view.focus = self.sheet_return;
        self.pressed = None;
    }
    fn key(&mut self, key: Key, down: bool, out: &mut Vec<Command>) -> bool {
        if !self.foreground {
            return false;
        }
        if down && self.idle_left().is_some() {
            self.idle_activity(out);
        }
        if self.pad_mode() == PadMode::Captured {
            // A lab test has every pad button; the remote's Back still leaves it.
            if key == Key::Back && !down {
                self.lab.capture = None;
                self.lab.flash = None;
                return true;
            }
            return false;
        }
        if self.view.page == Page::Running {
            // While a game runs, the pad is read by the engine; the remote never reaches the game, so its Back is the
            // only way to the overlay (and to ending the session) without a controller.
            if matches!(key, Key::Guide | Key::Back) && !down {
                self.open_overlay(out);
                return true;
            }
            return false;
        }
        if down {
            if matches!(key, Key::Up | Key::Down | Key::Left | Key::Right) {
                self.pressed = None;
                return self.move_focus(key, out);
            }
            if self.pressed.is_none() {
                self.pressed = Some((key, self.view.focus));
                return true;
            }
            return false;
        }
        let Some((pressed, at)) = self.pressed.take() else {
            return false;
        };
        if pressed != key || at != self.view.focus {
            return true;
        }
        match key {
            Key::Ok => self.activate(out),
            Key::Back => self.go_back(out),
            Key::Guide => self.open_overlay(out),
            Key::Menu if matches!(self.view.page, Page::Library | Page::Search) => {
                self.open_sheet(Layer::Sort, Focus::SheetRow(Sort::ALL.iter().position(|s| *s == self.view.sort).unwrap_or(0)))
            }
            Key::Menu if self.view.page == Page::Overlay && self.view.sheet.is_none() => self.set_stats(self.prefs.stats.mode.next(), out),
            Key::Alt if self.view.page == Page::Overlay && self.view.sheet.is_none() && self.steam_overlay() => self.open_steam_overlay(out),
            _ => {}
        }
        true
    }
    fn move_focus(&mut self, key: Key, out: &mut Vec<Command>) -> bool {
        if self.view.page == Page::Launch && self.view.focus == Focus::None {
            if let Some(n) = self.navigation.first() {
                self.view.focus = n.id;
                return true;
            }
        }
        let editable = match self.view.focus {
            Focus::EngineField(i) => self
                .view
                .selected_engine
                .as_ref()
                .and_then(|id| self.integration(id))
                .and_then(|e| e.schema.fields().nth(i))
                .is_some_and(|f| matches!(f.kind, FieldKind::Toggle { .. } | FieldKind::Choice { .. } | FieldKind::Number { .. })),
            Focus::Setting(i) => self.view.page != Page::Accounts || i == 0,
            _ => true,
        };
        if editable && matches!(key, Key::Left | Key::Right) && self.adjust(key == Key::Right, out) {
            return true;
        }
        let Some(n) = self.navigation.iter().find(|n| n.id == self.view.focus) else {
            return false;
        };
        let next = match key {
            Key::Up => n.up,
            Key::Down => n.down,
            Key::Left => n.left,
            Key::Right => n.right,
            _ => None,
        };
        if let Some(focus) = next {
            self.view.focus = focus;
            match focus {
                Focus::Home(row, _) => self.view.home_row = row,
                Focus::Grid(i) => {
                    let row = i / self.prefs.covers.columns();
                    if row != self.view.first_grid_row {
                        self.view.first_grid_row = row;
                    }
                }
                Focus::Rail(i) => self.view.place = settings::Place::ALL[i],
                Focus::Hero(_) => self.view.home_row = 0,
                _ => {}
            }
            return true;
        }
        false
    }
    fn activate(&mut self, out: &mut Vec<Command>) {
        match self.view.focus {
            Focus::Nav(i) => {
                self.back.clear();
                self.view.sheet = None;
                self.view.focus = Focus::Nav(i);
                self.view.page = match i {
                    1 => Page::Library,
                    2 => Page::Search,
                    3 => Page::Settings,
                    _ => Page::Home,
                };
                self.rebuild_filter();
            }
            Focus::None => {}
            f => Owner::of(self).activate(self, f, out),
        }
    }
    /// Left or Right on the focused control, by the rules of the slice that owns it.
    fn adjust(&mut self, right: bool, out: &mut Vec<Command>) -> bool {
        let f = self.view.focus;
        Owner::of(self).adjust(self, f, right, out)
    }
    fn save_prefs(&mut self, out: &mut Vec<Command>) {
        self.state.profile_mut().ui = self.prefs.json();
        out.push(Command::SaveState);
    }
    fn stats_request(&self, out: &mut Vec<Command>) {
        if let Some(s) = self.session.as_ref().filter(|s| *s.phase() == Phase::Running) {
            let pad_timing = self.hud_visible() || self.diagnostics.active();
            Self::send(out, &s.source.engine, Request::Stats { session: s.id, on: self.foreground && (pad_timing || self.view.page == Page::Overlay), pad_timing });
        }
    }
    /// Copies the values every engine shares (the stream's shape and the stick settings) into each enabled engine's settings where
    /// its schema takes them, and sends each its settings.
    fn push_common(&mut self, out: &mut Vec<Command>) {
        let desired = self.common_values();
        for engine in &self.integrations {
            if let Some(saved) = self.state.engines.get_mut(&engine.manifest.id).filter(|s| s.enabled) {
                for (key, value) in &desired {
                    if engine.schema.check(key, value).is_ok() {
                        saved.settings.insert(key.clone(), value.clone());
                    }
                }
                Self::send(out, &engine.manifest.id, Request::Settings { values: saved.settings.clone() });
            }
        }
    }
    fn common_values(&self) -> Values {
        Values::from([
            ("width".into(), Value::Int(i64::from(self.prefs.width))),
            ("height".into(), Value::Int(i64::from(self.prefs.height))),
            ("fps".into(), Value::Int(i64::from(self.prefs.fps))),
            ("hdr".into(), Value::Bool(self.prefs.hdr)),
            ("bitrate-mbps".into(), Value::Int(i64::from(self.stream_bitrate_mbps()))),
            ("codec".into(), Value::Text(self.prefs.codec.map_or("auto", Codec::as_str).into())),
            ("frame-pacing".into(), Value::Text(self.prefs.frame_pacing().into())),
            ("deadzone".into(), Value::Int(i64::from(self.prefs.deadzone))),
            ("anti-deadzone".into(), Value::Int(i64::from(self.prefs.anti_deadzone))),
            ("stick-curve".into(), Value::Text(self.prefs.stick_curve.name().into())),
        ])
    }
    /// The bitrate the engines are asked for: the viewer's choice, held to what the uplink carries. The choice itself stays as it was
    /// set, so a faster uplink lifts the limit by itself.
    pub fn stream_bitrate_mbps(&self) -> u32 {
        self.uplink.as_ref().map_or(self.prefs.bitrate_mbps, |uplink| uplink.limit(self.prefs.bitrate_mbps))
    }
    /// Gives each engine the bitrate the uplink now allows. An engine that runs hears it at once; the others take it from their saved
    /// settings when they start.
    fn follow_uplink(&mut self, out: &mut Vec<Command>) {
        let value = Value::Int(i64::from(self.stream_bitrate_mbps()));
        let running = self.foreground || self.session_active();
        for engine in &self.integrations {
            let Some(saved) = self.state.engines.get_mut(&engine.manifest.id) else { continue };
            if saved.settings.get("bitrate-mbps") == Some(&value) || engine.schema.check("bitrate-mbps", &value).is_err() {
                continue;
            }
            saved.settings.insert("bitrate-mbps".into(), value.clone());
            if saved.enabled && running && engine.descriptor.is_some() {
                Self::send(out, &engine.manifest.id, Request::Settings { values: saved.settings.clone() });
            }
        }
    }
    fn go_back(&mut self, out: &mut Vec<Command>) {
        if self.view.sheet.is_some() {
            if self.view.sheet == Some(Layer::EditText) && self.view.page == Page::Settings && self.view.place == settings::Place::Diagnostics { self.view.address.clear(); }
            if self.view.sheet == Some(Layer::Auth) {
                if let Some(a) = self.view.auth.take() {
                    Self::send(out, &a.engine, Request::AuthCancel);
                }
                if self.session_active() {
                    self.session_input(session::Input::Cancel, out);
                }
            }
            self.close_sheet();
            return;
        }
        match self.view.page {
            Page::Launch => self.session_input(session::Input::Cancel, out),
            Page::Running => {}
            Page::Overlay => self.resume(out),
            // Settings opened from the in-stream menu returns to it from any row.
            Page::Settings if matches!(self.back.last(), Some((Page::Overlay, _))) => {
                self.back.pop();
                self.view.page = Page::Overlay;
                self.view.focus = Focus::Overlay(1);
            }
            Page::Settings if matches!(self.view.focus, Focus::Setting(_) | Focus::Integration(_) | Focus::Theme(_)) => {
                self.view.focus = Focus::Rail(self.view.place.index());
            }
            Page::Hub if self.view.hub_tab != 0 => {
                self.view.hub_tab = 0;
                self.view.focus = Focus::Tab(0);
            }
            Page::Home if self.view.home_row > 0 => {
                self.view.home_row = 0;
                self.view.focus = Focus::Home(0, 0);
            }
            _ => {
                if let Some((page, focus)) = self.back.pop() {
                    self.view.page = page;
                    self.view.focus = focus;
                    self.rebuild_filter();
                } else if self.view.page != Page::Home {
                    self.view.page = Page::Home;
                    self.view.focus = Focus::Nav(0);
                } else {
                    out.push(Command::Exit);
                }
            }
        }
    }
    fn engine_event(&mut self, id: EngineId, event: ipc::Event, out: &mut Vec<Command>) {
        match event {
            ipc::Event::Ready { descriptor, schema } => {
                let first = self.integration(&id).is_some_and(|e| e.descriptor.is_none());
                if first {
                    let desired = self.common_values();
                    let saved = self.state.engines.entry(id.clone()).or_default();
                    for field in schema.fields() {
                        if let Some(value) = field.kind.default() {
                            saved.settings.entry(field.key.clone()).or_insert(value);
                        }
                    }
                    for (key, value) in desired {
                        if schema.check(&key, &value).is_ok() {
                            saved.settings.insert(key, value);
                        }
                    }
                    Self::send(out, &id, Request::Settings { values: saved.settings.clone() });
                }
                let auth = descriptor.capabilities.auth.first().copied();
                if self.view.page == Page::Accounts && self.view.selected_engine.as_ref() == Some(&id) {
                    if let Focus::EngineField(index) = self.view.focus {
                        let new_index = self.integration(&id).and_then(|e| e.schema.fields().nth(index))
                            .and_then(|old| schema.fields().position(|field| field.key == old.key && field.kind != FieldKind::Note));
                        let focus = new_index.map_or(Focus::Nav(3), Focus::EngineField);
                        if focus != self.view.focus {
                            self.view.focus = focus;
                            self.pressed = None;
                        }
                    }
                }
                if let Some(e) = self.integrations.iter_mut().find(|e| e.manifest.id == id) {
                    e.descriptor = Some(descriptor);
                    e.schema = schema;
                    e.error = None;
                }
                Self::send(out, &id, Request::Targets);
                if let (Some(a), Some(method)) = (self.view.auth.as_ref().filter(|a| a.engine == id && a.state.is_none()), auth) {
                    Self::send(out, &id, Request::Auth { target: a.target.clone(), method });
                }
            }
            ipc::Event::Targets { targets } => {
                // The engine answers every account refresh with Targets (start-up, sign-in, the membership tier); a traversal
                // already under way for a target is left to finish, since a second one shares its first cursor.
                for t in targets.iter().filter(|t| t.link == ipc::Link::Linked) {
                    if let std::collections::btree_map::Entry::Vacant(slot) = self.pages.entry((id.clone(), t.id.clone())) {
                        slot.insert(Vec::new());
                        Self::send(out, &id, Request::Library { target: t.id.clone(), cursor: None });
                    }
                }
                if let Some(e) = self.integrations.iter_mut().find(|e| e.manifest.id == id) {
                    e.targets = targets;
                }
                self.rebuild_catalog();
            }
            ipc::Event::Library { target, entries, next } => {
                // A page of a traversal the engine's error or restart ended is dropped: on its own it is not the library.
                let Some(page) = self.pages.get_mut(&(id.clone(), target.clone())) else { return };
                page.extend(entries);
                if next.is_some() {
                    Self::send(out, &id, Request::Library { target, cursor: next });
                } else {
                    let entries = self.pages.remove(&(id.clone(), target.clone())).unwrap_or_default();
                    self.listing.replace(&id, &target, entries);
                    let held = self.held();
                    self.rebuild_catalog();
                    self.follow(held);
                    self.refresh_loaded(&id);
                    out.push(Command::SaveCatalog);
                    out.push(Command::SaveState);
                }
            }
            ipc::Event::Readiness { target, game, readiness } => {
                self.listing.set_readiness(&SourceKey { engine: id, target, id: game }, readiness);
                self.rebuild_catalog();
            }
            ipc::Event::Auth { state } => {
                let ours = self.view.auth.as_ref().is_some_and(|a| a.engine == id);
                if !ours {
                    // An engine answers AuthCancel with a cancelled failure after the flow is gone; reopening the sheet then
                    // covered "Enter address" with "Link cancelled."
                    if matches!(state, AuthState::Failed { reason: ipc::AuthFail::Cancelled, .. }) {
                        return;
                    }
                    self.view.auth = Some(LinkFlow::new(id.clone(), None, self.now_ms));
                    self.open_sheet(Layer::Auth, Focus::SheetAction(0));
                }
                if matches!(state, AuthState::Done { .. }) {
                    if let Some(e) = self.integrations.iter_mut().find(|e| e.manifest.id == id && e.manifest.kind == EngineKind::Account)
                        && matches!(e.error, Some((ipc::ErrorCode::SignIn, _)))
                    {
                        e.error = None;
                    }
                    // The Targets answer starts the library traversal; a second one here overlapped it, and the GeForce NOW
                    // engine rejected the shared first cursor as repeated.
                    Self::send(out, &id, Request::Targets);
                    self.view.auth = None;
                    self.close_sheet();
                } else if let Some(a) = &mut self.view.auth {
                    a.expires_ms = match state {
                        AuthState::DeviceCode { expires_s, .. } | AuthState::Qr { expires_s, .. } => Some(self.now_ms + u64::from(expires_s) * 1000),
                        _ => None,
                    };
                    if matches!(state, AuthState::EnterPin { .. }) {
                        self.view.focus = Focus::Keyboard(0);
                    } else if a.no_pcs {
                        self.view.focus = Focus::SheetAction(0);
                    }
                    a.no_pcs = false;
                    a.state = Some(state);
                }
            }
            ipc::Event::Session { session, state } => {
                if self.session.as_ref().is_some_and(|s| s.id == session && s.source.engine == id) {
                    if let SessionState::Running { stream } = &state {
                        self.view.stream = stream.clone();
                    }
                    self.session_input(session::Input::Engine(state), out);
                }
            }
            ipc::Event::Stats { session, stats } => {
                if self.session.as_ref().is_some_and(|s| s.id == session && s.source.engine == id) {
                    self.telemetry.stream(stats.clone(), self.now_ms);
                    self.view.stats = stats;
                }
            }
            ipc::Event::Overlay { session } => {
                if self.session.as_ref().is_some_and(|s| s.id == session && s.source.engine == id) {
                    self.open_overlay(out);
                }
            }
            ipc::Event::IdleWarning { session, seconds } => {
                if self.session.as_ref().is_some_and(|s| s.id == session && s.source.engine == id) {
                    let sent_ms = self.view.idle.and_then(|w| w.sent_ms);
                    self.view.idle = seconds.map(|s| IdleWarning { ends_ms: self.now_ms + u64::from(s) * 1000, sent_ms });
                }
            }
            ipc::Event::Key { key, down } => {
                if down {
                    self.sources.used(Source::Controller);
                }
                if let Some(key) = Key::from_pad(key) {
                    self.key(key, down, out);
                }
            }
            ipc::Event::RoundTrip { session, trip } => {
                if self.lab.trip == Trip::Waiting(id.clone()) && self.session.as_ref().is_some_and(|s| s.id == session) {
                    self.lab.trip = Trip::Answer(id, trip);
                }
            }
            ipc::Event::Error { code, message } => {
                if self.lab.trip == Trip::Waiting(id.clone()) {
                    // An engine older than the request answers it as unknown; the lab reads that as "not supported".
                    if code == ipc::ErrorCode::Unsupported {
                        self.lab.trip = Trip::Unsupported(id);
                        // The Measure button goes with the answer, so focus moves to the first stick setting below it.
                        if self.view.focus == Focus::Lab(4) {
                            self.view.focus = Focus::Setting(0);
                        }
                        return;
                    }
                    self.lab.trip = Trip::Idle;
                }
                self.pages.retain(|(engine, _), _| *engine != id);
                self.refresh_error(&id, code);
                if self.engine_in_view(&id) {
                    self.view.notice = Some(message.clone());
                }
                if let Some(e) = self.integrations.iter_mut().find(|e| e.manifest.id == id) {
                    e.error = Some((code, message));
                }
            }
            ipc::Event::Refresh { step, note } => self.refresh_step(&id, step, note, out),
            ipc::Event::Achievements { target, game, unlocked } => self.owned_achievements(&id, &target, &game, unlocked),
            // The redesigned game page consumes these answers.
            ipc::Event::Community { .. } | ipc::Event::CommunityThread { .. } => {}
            ipc::Event::Unknown => {}
        }
    }
    fn request_visible_art(&mut self, out: &mut Vec<Command>) {
        let mut wanted = Vec::new();
        let hero = if matches!(self.view.page, Page::Home | Page::Library | Page::Search) { self.view.featured } else { self.view.selected };
        if let Some(id) = hero {
            for kind in [ArtKind::Hero, ArtKind::Logo] {
                if let Some(url) = self.art_url(id, kind) {
                    wanted.push(url);
                }
            }
        }
        if matches!(self.view.page, Page::Hub | Page::Media) {
            if let Some(id) = self.view.selected {
                if !self.details.contains_key(&id) && self.requested_details.insert(id) {
                    out.push(Command::FetchDetails { game: id });
                }
                if self.view.page == Page::Hub {
                    self.request_owned(id, out);
                }
                let media_open = self.view.page == Page::Media || self.hub_tab() == HubTab::Media;
                let icons_open = self.hub_tab() == HubTab::Achievements;
                if let Some(details) = self.details.get(&id) {
                    if media_open {
                        wanted.extend(details.media.iter().map(|m| m.image.clone()).filter(|url| !url.is_empty()));
                    }
                    if icons_open {
                        wanted.extend(details.hub.achievements.shown.iter().map(|a| a.icon.clone()));
                    }
                }
            }
        }
        let ids: Vec<GameId> = match self.view.page {
            Page::Home => {
                let mut ids = Vec::new();
                for (row_index, row) in self.home_rows.iter().enumerate().skip(self.view.home_row.saturating_sub(1)).take(4) {
                    let first = if let Focus::Home(r, i) = self.view.focus {
                        if r == row_index { i.saturating_sub(self.prefs.covers.columns() - 2) } else { 0 }
                    } else {
                        0
                    };
                    // Keep the adjacent tiles' art ready as they enter the clipped row.
                    for item in row.items.iter().skip(first.saturating_sub(1)).take(self.prefs.covers.columns() + 3) {
                        match item {
                            HomeItem::Game(id) => ids.push(*id),
                            HomeItem::Collection(index) => {
                                if let Some(id) = self.state.profile().user.collections.get(*index).and_then(|c| c.games.first()) {
                                    if let Some(url) = self.art_url(*id, ArtKind::Hero) {
                                        wanted.push(url);
                                    }
                                }
                            }
                            HomeItem::Genre(name) => ids.extend(self.genre_games.get(name).into_iter().flatten().take(3).copied()),
                        }
                    }
                }
                ids
            }
            Page::Library | Page::Search => {
                self.filtered.iter().skip(self.view.first_grid_row.saturating_sub(1) * self.prefs.covers.columns()).take(self.prefs.covers.columns() * 4).copied().collect()
            }
            _ => Vec::new(),
        };
        for id in ids {
            if let Some(url) = self.art_url(id, ArtKind::Cover) {
                wanted.push(url);
            }
        }
        for url in wanted {
            if !self.art.contains_key(&url) && self.requested_art.insert(url.clone()) {
                out.push(Command::FetchArt { url });
            }
        }
    }
    /// Explicit four-way links are rebuilt only when the destination changes. Endpoints have no neighbour: nothing wraps.
    pub fn rebuild_navigation(&mut self) {
        let mut rows: Vec<Vec<Focus>> = Vec::new();
        if let Some(layer) = self.view.sheet {
            rows = Owner::of_layer(layer).rows(self, Some(layer));
        } else {
            match self.view.page {
                Page::Home | Page::Library | Page::Search | Page::Settings | Page::Accounts | Page::Lab | Page::GameSir | Page::Leadjoy => rows.push((0..4).map(Focus::Nav).collect()),
                _ => {}
            }
            rows.extend(Owner::of_page(self.view.page).rows(self, None));
        }
        rows.retain(|r| !r.is_empty());
        self.navigation.clear();
        for (ri, row) in rows.iter().enumerate() {
            for (ci, id) in row.iter().enumerate() {
                let vertical = |r: &Vec<Focus>| r.get(ci.min(r.len() - 1)).copied();
                self.navigation.push(Neighbours {
                    id: *id,
                    left: ci.checked_sub(1).and_then(|i| row.get(i)).copied(),
                    right: row.get(ci + 1).copied(),
                    up: ri.checked_sub(1).and_then(|i| rows.get(i)).and_then(vertical),
                    down: rows.get(ri + 1).and_then(vertical),
                });
            }
        }
        if self.view.page == Page::Settings && self.view.sheet.is_none() {
            self.settings_links();
        }
        if !self.navigation.iter().any(|n| n.id == self.view.focus) {
            self.view.focus = if self.view.page == Page::Launch { Focus::None } else { self.navigation.first().map_or(Focus::None, |n| n.id) };
        }
    }
}

#[cfg(test)]
mod core_tests {
    use super::{tests::*, *};
    #[test]
    fn release_activates_only_the_destination_that_was_pressed() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        m.view.focus = Focus::Nav(1);
        m.step(Input::Key { key: Key::Ok, down: true });
        assert_eq!(m.view.page, Page::Home);
        m.step(Input::Key { key: Key::Right, down: true });
        m.step(Input::Key { key: Key::Ok, down: false });
        assert_eq!(m.view.page, Page::Home);
        key(&mut m, Key::Ok);
        assert_eq!(m.view.page, Page::Search);
    }
    #[test]
    fn navigation_has_no_wrap_and_repeated_presses_take_the_newest_destination() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        m.view.focus = Focus::Nav(0);
        key(&mut m, Key::Left);
        assert_eq!(m.view.focus, Focus::Nav(0));
        for _ in 0..8 {
            m.step(Input::Key { key: Key::Right, down: true });
        }
        assert_eq!(m.view.focus, Focus::Nav(3));
    }
}
