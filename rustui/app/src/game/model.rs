//! Pure navigation and application rules. Session decisions remain in lumagame-core; the shell only performs commands.
use lumagame_core::{
    catalog::{self, ArtKind, Game, Listing, SaveCompat},
    engine::{Codec, Descriptor, EngineKind, Feature, Manifest, Presentation},
    history::Played,
    ids::{EngineId, GameId, SessionId, SourceKey, TargetId},
    ipc::{self, AuthState, FromEngine, Readiness, Request, SessionState, Stats, StreamInfo, Target},
    launch::{self, Candidate, Resolution},
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Ok,
    Back,
    Menu,
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
        details: GameDetails,
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
}
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Send { engine: EngineId, request: Request },
    Enable { engine: EngineId, on: bool },
    KillEngine(EngineId),
    SaveState,
    SaveCatalog,
    Checkpoint(OpenSession),
    ClearCheckpoint,
    LowLatency(bool),
    FetchArt { url: String },
    FetchDetails { game: GameId },
    PlayMedia { url: String },
    OpenControllerSettings,
    Rumble { slot: u8, low: u16, high: u16 },
    GameSir(Option<ControllerChange>),
    Leadjoy { view: LeadjoyView, change: Option<ControllerChange> },
    Exit,
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
    pub reduce_motion: bool,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub hdr: bool,
    pub codec: Option<Codec>,
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
            reduce_motion: false,
            width: 3840,
            height: 2160,
            fps: 60,
            hdr: false,
            codec: None,
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
        p.bitrate_mbps = v["bitrate_mbps"].as_u64().unwrap_or(80).clamp(5, 150) as u32;
        p.stats = Overlay::read(&v["stats"]);
        p.deadzone = v["deadzone"].as_u64().map_or(p.deadzone, |n| n.min(u64::from(DEADZONE_MAX)) as u8);
        p.anti_deadzone = v["anti_deadzone"].as_u64().map_or(p.anti_deadzone, |n| n.min(u64::from(ANTI_DEADZONE_MAX)) as u8);
        p.stick_curve = v["stick_curve"].as_str().and_then(Curve::from_name).unwrap_or_default();
        p
    }
    fn json(&self) -> serde_json::Value {
        serde_json::json!({"family":self.family,"light":self.light,"hero":self.hero.index(),"covers":self.covers.index(),"text_percent":self.text_percent,"reduce_motion":self.reduce_motion,"width":self.width,"height":self.height,"fps":self.fps,"hdr":self.hdr,"codec":self.codec.map_or("auto",Codec::as_str),"bitrate_mbps":self.bitrate_mbps,"stats":self.stats,"deadzone":self.deadzone,"anti_deadzone":self.anti_deadzone,"stick_curve":self.stick_curve.name()})
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
    pub achievements: Option<String>,
    pub cloud_saves: Option<bool>,
    pub media: Vec<Media>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Media {
    pub title: String,
    pub image: String,
    pub video: Option<String>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Home,
    Library,
    Search,
    Hub,
    Settings,
    Integration,
    Launch,
    Running,
    Overlay,
    Media,
    Lab,
    GameSir,
    Leadjoy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sheet {
    Chooser,
    Sort,
    Genre,
    Theme,
    Auth,
    EndSession,
    AddHost,
    EditText,
    /// Confirms a controller setting that removes something, such as a macro.
    Delete,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Appearance,
    Streaming,
    Integrations,
    Controllers,
    About,
}
impl Section {
    pub const ALL: [Self; 5] = [Self::Appearance, Self::Streaming, Self::Integrations, Self::Controllers, Self::About];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
    pub fn name(self) -> &'static str {
        ["Appearance", "Streaming", "Integrations", "Controllers", "About"][self.index()]
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Focus {
    #[default]
    None,
    Nav(usize),
    Hero(usize),
    Home(usize, usize),
    Filter(usize),
    Grid(usize),
    SearchField,
    Keyboard(usize),
    HubTop(usize),
    HubAction(usize),
    Tab(usize),
    Way(usize),
    Media(usize),
    Rail(usize),
    Setting(usize),
    Theme(usize),
    Integration(usize),
    Target(usize),
    EngineField(usize),
    SheetRow(usize),
    Remember,
    SheetAction(usize),
    Overlay(usize),
    FirstRun(usize),
    /// The controller lab's actions: test buttons, rumble left, rumble right, flash test, measure the stream.
    Lab(usize),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Title,
    Recent,
    MostPlayed,
    Added,
}
impl Sort {
    pub const ALL: [Self; 4] = [Self::Title, Self::Recent, Self::MostPlayed, Self::Added];
    pub fn name(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Recent => "Recently played",
            Self::MostPlayed => "Most played",
            Self::Added => "Recently added",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub engine: Option<EngineId>,
    pub genre: Option<String>,
    pub favourites: bool,
    pub collection: Option<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HomeItem {
    Game(GameId),
    Collection(usize),
    Genre(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeRow {
    pub title: String,
    pub items: Vec<HomeItem>,
}
#[derive(Clone, Debug)]
pub struct Integration {
    pub manifest: Manifest,
    pub descriptor: Option<Descriptor>,
    pub schema: SettingsSchema,
    pub targets: Vec<Target>,
    pub error: Option<String>,
}
#[derive(Clone, Debug)]
pub struct LinkFlow {
    pub engine: EngineId,
    pub target: Option<TargetId>,
    pub state: Option<AuthState>,
    pub expires_ms: Option<u64>,
    pub pin: String,
    pub since_ms: u64,
    /// A PC engine searched for [`PC_SEARCH_MS`] without offering a PC to pair.
    pub no_pcs: bool,
}
/// How long "Pair a PC" waits for a PC engine to find one before offering an address and a retry.
pub const PC_SEARCH_MS: u64 = 20_000;
impl LinkFlow {
    fn new(engine: EngineId, target: Option<TargetId>, now_ms: u64) -> Self {
        Self { engine, target, state: None, expires_ms: None, pin: String::new(), since_ms: now_ms, no_pcs: false }
    }
}
/// "Play time on this TV": minutes under an hour, so ten minutes do not read as zero hours.
pub fn play_time(seconds: u64) -> String {
    match (seconds / 3600, seconds / 60 % 60) {
        (0, 0) => "Under a minute".into(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}
#[derive(Clone, Debug)]
pub struct View {
    pub page: Page,
    pub sheet: Option<Sheet>,
    pub focus: Focus,
    pub section: Section,
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
}
impl Default for View {
    fn default() -> Self {
        Self {
            page: Page::Home,
            sheet: None,
            focus: Focus::Hero(0),
            section: Section::Appearance,
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
        }
    }
}
/// How the shell reads the pads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadMode {
    // Discriminants are also the shell's atomic input mode.
    /// As the menu's keys.
    Menu,
    /// For the lab's live picture, and as the menu's keys without the sticks, which are under test.
    Lab,
    /// For the lab alone: a test has every button until B is held for [`LEAVE_HOLD_NS`] or the remote's Back is pressed.
    Captured,
}
/// A lab test that has every button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture {
    Buttons,
    /// The screen lights on the frame after a press, for a slow-motion video to time against the thumb.
    Flash,
}
/// The stream round trip the lab asked the running engine for.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Trip {
    #[default]
    Idle,
    Waiting(EngineId),
    Answer(EngineId, ipc::RoundTrip),
    /// The engine does not time it: no `round-trip` feature, or an older engine that cannot read the request.
    Unsupported(EngineId),
}
/// Holding B this long leaves a test that has every button.
pub const LEAVE_HOLD_NS: u64 = 1_000_000_000;
/// The lab's figures change at most this often; the dots and bars follow every report.
pub const LAB_REFRESH_NS: u64 = 100_000_000;
#[derive(Clone, Debug, Default)]
pub struct LabPad {
    pub name: String,
    /// The latest report, before any dead zone.
    pub report: RawReport,
    pub timing: Timing,
    pub summary: Summary,
    /// Left and right stick.
    pub drift: [Drift; 2],
    /// The kernel's stamp of a B press still held.
    b_down: Option<u64>,
}
#[derive(Clone, Debug, Default)]
pub struct Lab {
    pub pads: [Option<LabPad>; MAX_PADS],
    /// The pad that reported last, which the lab draws.
    pub active: Option<usize>,
    pub capture: Option<Capture>,
    /// The kernel's stamp of the press that lit the flash, while any button is held.
    pub flash: Option<u64>,
    /// The number of the frame the shell draws next, kept while the flash test runs.
    pub frame: u64,
    pub trip: Trip,
    refreshed_ns: u64,
}
impl Lab {
    pub fn connected(&self) -> usize {
        self.pads.iter().flatten().count()
    }
    /// The pad the lab draws: the one that reported last, else the first connected.
    pub fn pad(&self) -> Option<&LabPad> {
        self.active.and_then(|i| self.pads[i].as_ref()).or_else(|| self.pads.iter().flatten().next())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Neighbours {
    pub id: Focus,
    pub up: Option<Focus>,
    pub down: Option<Focus>,
    pub left: Option<Focus>,
    pub right: Option<Focus>,
}
#[derive(Clone, Debug)]
pub struct Model {
    state: State,
    listing: Listing,
    games: Vec<Game>,
    pub integrations: Vec<Integration>,
    pub prefs: Preferences,
    pub view: View,
    pub details: BTreeMap<GameId, GameDetails>,
    pub art: BTreeMap<String, PathBuf>,
    pub home_rows: Vec<HomeRow>,
    pub filtered: Vec<GameId>,
    pub navigation: Vec<Neighbours>,
    pub lab: Lab,
    pub gamesir: GameSir,
    pub leadjoy: Leadjoy,
    pub telemetry: Telemetry,
    session: Option<Session>,
    session_start: u64,
    foreground: bool,
    now_ms: u64,
    unix_s: u64,
    revision: u64,
    next_session: u64,
    pressed: Option<(Key, Focus)>,
    back: Vec<(Page, Focus)>,
    sheet_return: Focus,
    overlay_return: bool,
    closing: bool,
    requested_art: BTreeSet<String>,
    requested_details: BTreeSet<GameId>,
    pages: BTreeMap<(EngineId, TargetId), Vec<ipc::LibraryEntry>>,
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
                    _ => 2,
                });
                list
            },
            prefs,
            view: View::default(),
            details: BTreeMap::new(),
            art: BTreeMap::new(),
            home_rows: Vec::new(),
            filtered: Vec::new(),
            navigation: Vec::new(),
            lab: Lab::default(),
            gamesir: GameSir::default(),
            leadjoy: Leadjoy::default(),
            telemetry: Telemetry::default(),
            session: None,
            session_start: 0,
            foreground: true,
            now_ms: 0,
            unix_s: 0,
            revision: 1,
            next_session: 1,
            pressed: None,
            back: Vec::new(),
            sheet_return: Focus::None,
            overlay_return: false,
            closing: false,
            requested_art: BTreeSet::new(),
            requested_details: BTreeSet::new(),
            pages: BTreeMap::new(),
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
    pub fn foreground(&self) -> bool {
        self.foreground
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }
    pub fn session_active(&self) -> bool {
        self.session.as_ref().is_some_and(|s| !s.is_done())
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
        self.foreground && (self.view.page != Page::Running || self.hud_visible())
    }
    pub fn hud_visible(&self) -> bool {
        self.foreground && self.view.page == Page::Running && self.streaming() && self.prefs.stats.mode.on()
    }
    /// How the shell should read the pads: the lab's own modes while its page has them and no sheet covers it.
    pub fn pad_mode(&self) -> PadMode {
        if !self.wants_pads() || self.view.page != Page::Lab || self.view.sheet.is_some() {
            PadMode::Menu
        } else if self.lab.capture.is_some() {
            PadMode::Captured
        } else {
            PadMode::Lab
        }
    }
    /// The kernel's stamp of the press the flash test shows lit, and the number of the frame that shows it.
    pub fn flash_press(&self) -> Option<(u64, u64)> {
        if self.pad_mode() != PadMode::Captured || self.lab.capture != Some(Capture::Flash) {
            return None;
        }
        self.lab.flash.map(|press| (press, self.lab.frame))
    }
    /// The running session's engine, when it can time the stream's round trip for the lab.
    pub fn round_trip_engine(&self) -> Option<&EngineId> {
        let s = self.session.as_ref().filter(|s| !s.is_done())?;
        let d = self.integration(&s.source.engine)?.descriptor.as_ref()?;
        d.capabilities.features.contains(&Feature::RoundTrip).then_some(&s.source.engine)
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
    pub fn candidates(&self, game: GameId) -> Vec<Candidate> {
        self.game(game).map(|g| g.sources.iter().map(|s| Candidate::of(s, self.target(&s.key()))).collect()).unwrap_or_default()
    }
    pub fn resolution(&self, game: GameId) -> Resolution {
        launch::resolve(&self.candidates(game), self.state.profile().user.remembered(game))
    }
    pub fn play_target(&self, game: GameId) -> Option<SourceKey> {
        if self.view.selected == Some(game) {
            if let Some(k) = &self.view.chooser_selected {
                return Some(k.clone());
            }
        }
        match self.resolution(game) {
            Resolution::Launch { key, .. } => Some(key),
            Resolution::Choose { focus, .. } => focus,
            Resolution::Nothing => None,
        }
    }
    pub fn readiness(&self, key: &SourceKey) -> Readiness {
        self.games.iter().flat_map(|g| &g.sources).find(|s| s.key() == *key).map_or(Readiness::Offline, |s| Candidate::of(s, self.target(key)).readiness)
    }
    pub fn save_compat(&self, key: &SourceKey) -> SaveCompat {
        let Some(game) = self.selected_game() else {
            return SaveCompat::Unknown;
        };
        let baseline = self.view.chooser_selected.as_ref().and_then(|k| game.source(k)).or_else(|| game.sources.first());
        match (baseline, game.source(key)) {
            (Some(a), Some(b)) => catalog::save_compat(&a.entry, &b.entry, self.details.get(&game.id).and_then(|d| d.cloud_saves)),
            _ => SaveCompat::Unknown,
        }
    }
    pub fn elapsed(&self) -> u32 {
        self.session.as_ref().and_then(|s| s.played(self.now_ms)).unwrap_or(0)
    }
    pub fn auth_remaining(&self) -> Option<u64> {
        self.view.auth.as_ref()?.expires_ms.map(|end| end.saturating_sub(self.now_ms).div_ceil(1000))
    }
    /// The end-session sheet's title, explanation and confirming button, in the words of the engine's kind.
    pub fn end_copy(&self) -> (&'static str, String, &'static str) {
        let engine = self.session.as_ref().map(|s| &s.source.engine);
        let kind = engine.and_then(|id| self.integration(id)).map(|e| e.manifest.kind);
        let place = match (kind, engine) {
            (Some(EngineKind::Host), _) => "on the PC".to_string(),
            (Some(EngineKind::Cloud), Some(id)) => format!("on {}", self.engine_name(id)),
            _ => String::new(),
        };
        if self.can_end_game() {
            let body = if place.is_empty() { "The game will close.".to_string() } else { format!("The game will close {place}.") };
            ("End this session?", format!("{body} Unsaved progress may be lost."), "End session")
        } else {
            let body = if place.is_empty() { "The game can keep running.".to_string() } else { format!("The game can keep running {place}.") };
            ("Leave the stream?", format!("The stream will close. {body}"), "Leave stream")
        }
    }
    pub fn since_played(&self, id: GameId) -> Option<String> {
        let total = self.state.profile().history.total(id);
        if total.sessions == 0 {
            return None;
        }
        let seconds = self.unix_s.saturating_sub(total.last);
        Some(if seconds < 60 {
            "Just now".into()
        } else if seconds < 3600 {
            format!("{} min ago", seconds / 60)
        } else if seconds < 86400 {
            format!("{} h ago", seconds / 3600)
        } else {
            format!("{} days ago", seconds / 86400)
        })
    }
    pub fn genres(&self) -> Vec<String> {
        self.details.values().flat_map(|d| d.genres.iter().cloned()).collect::<BTreeSet<_>>().into_iter().collect()
    }
    pub fn filter_count(&self, engine: Option<&EngineId>) -> usize {
        self.games
            .iter()
            .filter(|g| !self.state.profile().user.hidden.contains(&g.id) && engine.is_none_or(|e| g.sources.iter().any(|s| &s.engine == e)))
            .count()
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
            Input::Key { key, down } => changed = self.key(key, down, &mut out),
            Input::Click { focus, down } => {
                if self.navigation.iter().any(|n| n.id == focus) {
                    self.view.focus = focus;
                    changed = self.key(Key::Ok, down, &mut out);
                }
            }
            Input::Engine { engine, message } => {
                changed = self.view.page != Page::Running || !matches!(message.event, ipc::Event::Stats { .. });
                self.engine_event(engine, message.event, &mut out);
            }
            Input::EngineGone(id) => {
                self.pages.retain(|(engine, _), _| *engine != id);
                if let Some(e) = self.integrations.iter_mut().find(|e| e.manifest.id == id) {
                    e.descriptor = None;
                    e.error = Some("Connection lost".into());
                }
                if self.session.as_ref().is_some_and(|s| s.source.engine == id) {
                    self.session_input(session::Input::EngineGone, &mut out);
                }
            }
            Input::Foreground(on) => {
                self.foreground = on;
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
                changed = second && (self.view.page == Page::Overlay || self.view.auth.as_ref().is_some_and(|a| a.expires_ms.is_some()));
                if self.hud_visible() { changed |= self.telemetry.tick(now_ms); }
                if self.view.auth.as_ref().is_some_and(|a| a.expires_ms.is_some_and(|end| now_ms >= end) && !matches!(a.state, Some(AuthState::Failed { .. })))
                {
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
                self.details.insert(game, details);
                self.rebuild_rows();
                self.rebuild_filter();
            }
            Input::Text(text) => match self.view.sheet {
                Some(Sheet::Auth) => {
                    if let Some(a) = &mut self.view.auth {
                        let max = match a.state {
                            Some(AuthState::EnterPin { digits }) => usize::from(digits),
                            _ => 12,
                        };
                        a.pin.extend(text.chars().filter(char::is_ascii_digit).take(max.saturating_sub(a.pin.len())));
                    }
                }
                Some(Sheet::AddHost | Sheet::EditText) => {
                    self.view.address.extend(text.chars().filter(|c| !c.is_control()).take(255usize.saturating_sub(self.view.address.chars().count())))
                }
                _ => {
                    self.view.query.extend(text.chars().filter(|c| !c.is_control()).take(128usize.saturating_sub(self.view.query.chars().count())));
                    self.rebuild_filter();
                }
            },
            Input::Lab { now_ns, reports, names, frame } => changed = self.lab_input(now_ns, reports, names, frame),
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
                        if let LeadjoyState::Ready(profile) = &state { self.leadjoy.view = profile.view.clone(); }
                        self.leadjoy.state = state; self.leadjoy.error = None;
                    }
                    Err(error) => {
                        self.leadjoy.error = Some(error);
                        if self.view.page == Page::Leadjoy && self.view.sheet.is_none() { self.view.focus = Focus::Setting(0); }
                    }
                }
            }
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
    /// The lab's share of the pads. True when its page shows something new.
    fn lab_input(&mut self, now_ns: u64, reports: Vec<(u8, RawReport)>, names: [Option<String>; MAX_PADS], frame: u64) -> bool {
        if self.view.page != Page::Lab {
            return false;
        }
        let lab = &mut self.lab;
        let mut changed = false;
        for (pad, name) in lab.pads.iter_mut().zip(names) {
            if pad.as_ref().map(|p| &p.name) != name.as_ref() {
                *pad = name.map(|name| LabPad { name, ..LabPad::default() });
                changed = true;
            }
        }
        if lab.active.is_some_and(|i| lab.pads[i].is_none()) {
            lab.active = None;
        }
        let mut new_flash = false;
        for (slot, report) in reports {
            let Some(pad) = lab.pads.get_mut(usize::from(slot)).and_then(Option::as_mut) else {
                continue;
            };
            if lab.capture == Some(Capture::Flash) && lab.flash.is_none() && report.buttons & !pad.report.buttons != 0 {
                lab.flash = Some(report.kernel_ns);
                new_flash = true;
            }
            pad.b_down = if report.buttons & button::B == 0 { None } else { pad.b_down.or(Some(report.kernel_ns)) };
            pad.timing.push(report.kernel_ns);
            pad.drift[0].report(report.left, report.kernel_ns);
            pad.drift[1].report(report.right, report.kernel_ns);
            pad.report = report;
            lab.active = Some(usize::from(slot));
            changed = true;
        }
        if !new_flash && lab.frame != frame && lab.flash.is_some() && lab.pads.iter().flatten().all(|p| p.report.buttons == 0) {
            lab.flash = None;
        }
        for pad in lab.pads.iter_mut().flatten() {
            for drift in &mut pad.drift {
                changed |= drift.tick(now_ns);
            }
        }
        if now_ns >= lab.refreshed_ns + LAB_REFRESH_NS {
            lab.refreshed_ns = now_ns;
            for pad in lab.pads.iter_mut().flatten() {
                let summary = pad.timing.summary(now_ns);
                changed |= summary != pad.summary;
                pad.summary = summary;
            }
        }
        if lab.capture.is_some() && lab.pads.iter().flatten().any(|p| p.b_down.is_some_and(|at| now_ns.saturating_sub(at) >= LEAVE_HOLD_NS)) {
            lab.capture = None;
            lab.flash = None;
            changed = true;
        }
        if lab.capture == Some(Capture::Flash) && lab.frame != frame {
            lab.frame = frame;
            changed = true;
        }
        changed
    }
    fn lab_action(&mut self, action: usize, out: &mut Vec<Command>) {
        match action {
            0 => self.lab.capture = Some(Capture::Buttons),
            1 | 2 => {
                // The left motor is the heavy, low one.
                let (low, high) = if action == 1 { (0xC000, 0) } else { (0, 0xC000) };
                let slots = (0..MAX_PADS).filter(|&i| self.lab.pads[i].is_some()).map(|i| i as u8);
                let before = out.len();
                out.extend(slots.map(|slot| Command::Rumble { slot, low, high }));
                if out.len() == before {
                    self.view.notice = Some("No controller is connected.".into());
                }
            }
            3 => {
                self.lab.capture = Some(Capture::Flash);
                self.lab.flash = None;
            }
            4 => {
                if let (Some(engine), Some(session)) = (self.round_trip_engine().cloned(), self.session.as_ref().map(|s| s.id)) {
                    self.lab.trip = Trip::Waiting(engine.clone());
                    Self::send(out, &engine, Request::RoundTrip { session });
                }
            }
            _ => {}
        }
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
            self.view.featured =
                self.state.profile().history.recently_played().into_iter().find(|id| self.game(*id).is_some()).or_else(|| self.games.first().map(|g| g.id));
        }
        self.rebuild_rows();
        self.rebuild_filter();
    }
    fn rebuild_rows(&mut self) {
        let profile = self.state.profile();
        let valid = |id: &GameId| self.games.iter().any(|g| g.id == *id) && !profile.user.hidden.contains(id);
        let mut rows = Vec::new();
        let recent = profile.history.recently_played().into_iter().filter(valid).map(HomeItem::Game).collect::<Vec<_>>();
        if !recent.is_empty() {
            rows.push(HomeRow { title: "Continue".into(), items: recent });
        }
        let most = profile.history.most_played().into_iter().filter(valid).map(HomeItem::Game).collect::<Vec<_>>();
        if !most.is_empty() {
            rows.push(HomeRow { title: "Most played".into(), items: most });
        }
        let favourites = profile.user.favourites.iter().filter(|id| valid(id)).copied().map(HomeItem::Game).collect::<Vec<_>>();
        if !favourites.is_empty() {
            rows.push(HomeRow { title: "Favourites".into(), items: favourites });
        }
        let collections =
            profile.user.collections.iter().enumerate().filter(|(_, c)| c.games.iter().any(valid)).map(|(i, _)| HomeItem::Collection(i)).collect::<Vec<_>>();
        if !collections.is_empty() {
            rows.push(HomeRow { title: "Collections".into(), items: collections });
        }
        let genres = self.genres().into_iter().map(HomeItem::Genre).collect::<Vec<_>>();
        if !genres.is_empty() {
            rows.push(HomeRow { title: "Genres".into(), items: genres });
        }
        let native = self
            .games
            .iter()
            .filter(|g| valid(&g.id) && g.sources.iter().any(|s| self.integration(&s.engine).is_some_and(|e| e.manifest.kind == EngineKind::Native)))
            .map(|g| HomeItem::Game(g.id))
            .collect::<Vec<_>>();
        if !native.is_empty() {
            rows.push(HomeRow { title: "Native on this TV".into(), items: native });
        }
        if rows.is_empty() && !self.games.is_empty() {
            rows.push(HomeRow { title: "Library".into(), items: self.games.iter().filter(|g| valid(&g.id)).map(|g| HomeItem::Game(g.id)).collect() });
        }
        self.home_rows = rows;
        self.view.home_row = self.view.home_row.min(self.home_rows.len().saturating_sub(1));
    }
    fn rebuild_filter(&mut self) {
        let p = self.state.profile();
        let f = &self.view.filter;
        let query = self.view.query.to_lowercase();
        self.filtered = self
            .games
            .iter()
            .filter(|g| {
                !p.user.hidden.contains(&g.id)
                    && (self.view.page != Page::Search || g.title.to_lowercase().contains(&query))
                    && f.engine.as_ref().is_none_or(|id| g.sources.iter().any(|s| &s.engine == id))
                    && (!f.favourites || p.user.favourites.contains(&g.id))
                    && f.collection.is_none_or(|i| p.user.collections.get(i).is_some_and(|c| c.games.contains(&g.id)))
                    && f.genre.as_ref().is_none_or(|genre| self.details.get(&g.id).is_some_and(|d| d.genres.contains(genre)))
            })
            .map(|g| g.id)
            .collect();
        match self.view.sort {
            Sort::Title => {}
            Sort::Recent => self.filtered.sort_by_key(|id| std::cmp::Reverse(p.history.total(*id).last)),
            Sort::MostPlayed => self.filtered.sort_by_key(|id| std::cmp::Reverse(p.history.total(*id).seconds)),
            Sort::Added => self.filtered.sort_by_key(|id| std::cmp::Reverse(id.0)),
        }
        self.view.first_grid_row = self.view.first_grid_row.min(self.filtered.len().saturating_sub(1) / self.prefs.covers.columns());
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
    fn open_sheet(&mut self, sheet: Sheet, focus: Focus) {
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
    fn hub(&mut self, id: GameId) {
        self.view.selected = Some(id);
        self.view.chooser_selected = None;
        self.view.hub_tab = 0;
        self.view.notice = None;
        self.show_page(Page::Hub, Focus::HubAction(0));
    }
    fn key(&mut self, key: Key, down: bool, out: &mut Vec<Command>) -> bool {
        if !self.foreground {
            return false;
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
                self.open_sheet(Sheet::Sort, Focus::SheetRow(Sort::ALL.iter().position(|s| *s == self.view.sort).unwrap_or(0)))
            }
            Key::Menu if self.view.page == Page::Overlay && self.view.sheet.is_none() => self.set_stats(self.prefs.stats.mode.next(), out),
            _ => {}
        }
        true
    }
    fn move_focus(&mut self, key: Key, out: &mut Vec<Command>) -> bool {
        let editable = match self.view.focus {
            Focus::EngineField(i) => self
                .view
                .selected_engine
                .as_ref()
                .and_then(|id| self.integration(id))
                .and_then(|e| e.schema.fields().nth(i))
                .is_some_and(|f| matches!(f.kind, FieldKind::Toggle { .. } | FieldKind::Choice { .. } | FieldKind::Number { .. })),
            Focus::Setting(i) => self.view.page != Page::Integration || i == 0,
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
                Focus::Rail(i) => self.view.section = Section::ALL[i],
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
            Focus::Hero(0) => {
                if let Some(id) = self.view.featured {
                    self.view.selected = Some(id);
                    self.play(out);
                }
            }
            Focus::Hero(_) => {
                if let Some(id) = self.view.featured {
                    self.hub(id);
                }
            }
            Focus::Home(r, i) => match self.home_rows.get(r).and_then(|r| r.items.get(i)).cloned() {
                Some(HomeItem::Game(id)) => self.hub(id),
                Some(HomeItem::Collection(i)) => {
                    self.view.filter = Filter { collection: Some(i), ..Filter::default() };
                    self.show_page(Page::Library, Focus::Grid(0));
                    self.rebuild_filter();
                }
                Some(HomeItem::Genre(genre)) => {
                    self.view.filter = Filter { genre: Some(genre), ..Filter::default() };
                    self.show_page(Page::Library, Focus::Grid(0));
                    self.rebuild_filter();
                }
                _ => {}
            },
            Focus::Grid(i) => {
                if let Some(id) = self.filtered.get(i).copied() {
                    self.hub(id);
                }
            }
            Focus::Filter(i) => {
                let engines = self.integrations.iter().filter(|e| self.enabled(&e.manifest.id)).map(|e| e.manifest.id.clone()).collect::<Vec<_>>();
                if i == 0 {
                    self.view.filter = Filter::default();
                } else if let Some(id) = engines.get(i - 1) {
                    self.view.filter.engine = Some(id.clone());
                } else if i == engines.len() + 1 {
                    self.open_sheet(Sheet::Genre, Focus::SheetRow(0));
                } else if i == engines.len() + 2 {
                    self.view.filter.favourites = !self.view.filter.favourites;
                } else {
                    self.open_sheet(Sheet::Sort, Focus::SheetRow(Sort::ALL.iter().position(|s| *s == self.view.sort).unwrap_or(0)));
                }
                self.view.first_grid_row = 0;
                self.rebuild_filter();
            }
            Focus::SearchField => self.view.focus = Focus::Keyboard(0),
            Focus::Keyboard(i) => self.keyboard(i, out),
            Focus::HubTop(0) => self.go_back(out),
            Focus::HubTop(1) => {
                if let Some(id) = self.view.selected {
                    let f = &mut self.state.profile_mut().user.favourites;
                    if !f.remove(&id) {
                        f.insert(id);
                    }
                    self.rebuild_rows();
                    out.push(Command::SaveState);
                }
            }
            Focus::HubTop(_) => self.open_chooser(),
            Focus::HubAction(0) => self.play(out),
            Focus::HubAction(_) => self.open_chooser(),
            Focus::Tab(tab) => {
                self.view.hub_tab = tab;
            }
            Focus::Way(i) => {
                if let Some(game) = self.selected_game() {
                    if let Some(s) = game.sources.get(i) {
                        self.view.chooser_selected = Some(s.key());
                        self.view.hub_tab = 0;
                        self.view.focus = Focus::HubAction(0);
                    } else {
                        self.view.section = Section::Integrations;
                        self.show_page(Page::Settings, Focus::Integration(0));
                    }
                }
            }
            Focus::Media(i) => {
                if let Some(media) = self.view.selected.and_then(|id| self.details.get(&id)).and_then(|d| d.media.get(i)) {
                    if let Some(url) = &media.video {
                        out.push(Command::PlayMedia { url: url.clone() });
                    } else {
                        self.view.media = i;
                        self.show_page(Page::Media, Focus::HubTop(0));
                    }
                }
            }
            Focus::Rail(i) => {
                self.view.section = Section::ALL[i];
                self.view.focus = if self.view.section == Section::Integrations { Focus::Integration(0) } else { Focus::Setting(0) };
            }
            Focus::Setting(i) => {
                if matches!(self.view.page, Page::Integration | Page::Lab | Page::GameSir | Page::Leadjoy) {
                    self.adjust(true, out);
                } else if self.view.section == Section::Appearance && i == 0 {
                    self.open_sheet(Sheet::Theme, Focus::Theme(0));
                } else if self.view.section == Section::Controllers {
                    if i == 0 {
                        self.lab = Lab::default();
                        let focus = if !self.session_active() {
                            Focus::Lab(0)
                        } else if self.round_trip_engine().is_some() {
                            Focus::Lab(4)
                        } else {
                            Focus::Setting(0)
                        };
                        self.show_page(Page::Lab, focus);
                    } else if i == 1 {
                        out.push(Command::OpenControllerSettings);
                    } else if i == 2 {
                        self.show_page(Page::GameSir, Focus::Setting(0));
                        self.read_gamesir(out);
                    } else {
                        self.show_page(Page::Leadjoy, Focus::Setting(0));
                        self.read_leadjoy(out);
                    }
                } else if self.view.section == Section::About {
                    out.push(Command::PlayMedia { url: ["https://store.steampowered.com", "https://steamgriddb.com", "https://igdb.com"][i.min(2)].into() });
                } else {
                    self.adjust(true, out);
                }
            }
            Focus::Lab(i) => self.lab_action(i, out),
            Focus::Theme(i) => {
                let families = lumatv_tokens::themes::game_catalog();
                self.prefs.family = if i == 0 { "match-lumatv".into() } else { families.get(i - 1).map_or("rose-pine", |f| f.id).into() };
                self.save_prefs(out);
                if self.view.sheet.is_some() {
                    self.close_sheet();
                }
            }
            Focus::FirstRun(i) | Focus::Integration(i) => {
                if let Some(e) = self.integrations.get(i) {
                    self.view.selected_engine = Some(e.manifest.id.clone());
                    self.show_page(Page::Integration, Focus::Setting(0));
                }
            }
            Focus::Target(i) => {
                if let Some(id) = self.view.selected_engine.clone() {
                    let target = self.integration(&id).and_then(|e| e.targets.get(i)).map(|t| t.id.clone());
                    self.begin_auth(id, target, out);
                }
            }
            Focus::EngineField(i) => self.change_engine_field(i, true, out),
            Focus::SheetRow(i) => match self.view.sheet {
                Some(Sheet::Chooser) => {
                    if let Some(s) = self.selected_game().and_then(|g| g.sources.get(i)) {
                        self.view.chooser_selected = Some(s.key());
                        self.view.focus = Focus::SheetAction(0);
                    }
                }
                Some(Sheet::Sort) => {
                    if let Some(sort) = Sort::ALL.get(i) {
                        self.view.sort = *sort;
                    }
                    self.rebuild_filter();
                    self.close_sheet();
                }
                Some(Sheet::Genre) => {
                    self.view.filter.genre = if i == 0 { None } else { self.genres().get(i - 1).cloned() };
                    self.rebuild_filter();
                    self.close_sheet();
                }
                _ => {}
            },
            Focus::Remember => self.view.remember = !self.view.remember,
            Focus::SheetAction(i) => match self.view.sheet {
                Some(Sheet::Chooser) => {
                    if let (Some(game), Some(source)) = (self.view.selected, self.view.chooser_selected.clone()) {
                        if self.readiness(&source).can_attempt() {
                            self.start(game, source, self.view.remember, out);
                        } else {
                            self.view.notice = Some("This target is unavailable. Choose another way to play.".into());
                        }
                    }
                }
                Some(Sheet::EndSession) => {
                    if i == 0 {
                        self.close_sheet();
                        self.resume(out);
                    } else {
                        self.close_sheet();
                        self.session_input(session::Input::Stop { end_game: self.can_end_game() }, out);
                    }
                }
                Some(Sheet::Auth) => self.auth_action(i, out),
                Some(Sheet::EditText) => self.save_text(out),
                Some(Sheet::Delete) => {
                    if i == 1 { self.delete_leadjoy(out); }
                    self.close_sheet();
                }
                Some(Sheet::AddHost) if !self.view.address.is_empty() => {
                    if let Some(id) = &self.view.selected_engine {
                        Self::send(out, id, Request::AddTarget { address: self.view.address.clone() });
                    }
                    self.close_sheet();
                }
                _ => {}
            },
            Focus::Overlay(i) => match i {
                0 => self.resume(out),
                1 => {
                    self.overlay_return = true;
                    self.view.section = Section::Streaming;
                    self.show_page(Page::Settings, Focus::Setting(0));
                }
                2 => self.set_stats(self.prefs.stats.mode.next(), out),
                _ => self.open_sheet(Sheet::EndSession, Focus::SheetAction(0)),
            },
            Focus::None => {}
        }
    }
    fn read_gamesir(&mut self, out: &mut Vec<Command>) {
        if !self.gamesir.pending {
            self.gamesir.pending = true;
            self.gamesir.error = None;
            out.push(Command::GameSir(None));
        }
    }
    fn adjust_gamesir(&mut self, row: usize, right: bool, out: &mut Vec<Command>) -> bool {
        if self.gamesir.pending { return true; }
        if row == 0 {
            self.read_gamesir(out);
            return true;
        }
        if self.gamesir.error.is_some() { return true; }
        let GameSirState::Ready(profile) = &self.gamesir.state else { return false };
        let values = &profile.values;
        let nudge = |value: u8, max| if right { value.saturating_add(1).min(max) } else { value.saturating_sub(1) };
        let (setting, value, current) = match row {
            1 => {
                let rates = [250, 500, 1000];
                let at = values.report_rate.and_then(|rate| rates.iter().position(|r| *r == rate));
                let next = at.map_or(0, |at| if right { (at + 1).min(2) } else { at.saturating_sub(1) });
                ("report-rate", serde_json::json!(rates[next]), serde_json::json!(values.report_rate))
            }
            2 => ("rumble-left", serde_json::json!(nudge(values.rumble_left, 4)), serde_json::json!(values.rumble_left)),
            3 => ("rumble-right", serde_json::json!(nudge(values.rumble_right, 4)), serde_json::json!(values.rumble_right)),
            4 => ("dpad-diagonal-lock", serde_json::json!(!values.dpad_diagonal_lock), serde_json::json!(values.dpad_diagonal_lock)),
            5 => ("xinput-abxy-swap", serde_json::json!(!values.xinput_abxy_swap), serde_json::json!(values.xinput_abxy_swap)),
            6 => ("deadzone-left", serde_json::json!(nudge(values.deadzone_left, 20)), serde_json::json!(values.deadzone_left)),
            7 => ("deadzone-right", serde_json::json!(nudge(values.deadzone_right, 20)), serde_json::json!(values.deadzone_right)),
            _ => return false,
        };
        if value != current {
            self.gamesir.pending = true;
            out.push(Command::GameSir(Some(ControllerChange { setting: setting.into(), value })));
        }
        true
    }
    fn read_leadjoy(&mut self, out: &mut Vec<Command>) {
        if !self.leadjoy.pending {
            self.leadjoy.pending = true;
            self.leadjoy.error = None;
            out.push(Command::Leadjoy { view: self.leadjoy.view.clone(), change: None });
        }
    }
    fn adjust_leadjoy(&mut self, row: usize, right: bool, out: &mut Vec<Command>) -> bool {
        if self.leadjoy.pending { return true; }
        if row == 0 { self.read_leadjoy(out); return true; }
        if self.leadjoy.error.is_some() { return true; }
        let LeadjoyState::Ready(profile) = &self.leadjoy.state else { return false };
        let Some(field) = profile.schema.fields().nth(row - 1) else { return false };
        if field.kind == FieldKind::Action {
            if field.key.starts_with("delete-") {
                self.view.edit_field = Some(row - 1);
                self.open_sheet(Sheet::Delete, Focus::SheetAction(0));
                return true;
            }
            let change = ControllerChange { setting: field.key.clone().into(), value: serde_json::Value::Null };
            self.leadjoy.pending = true;
            out.push(Command::Leadjoy { view: profile.view.clone(), change: Some(change) });
            return true;
        }
        let Some(current) = profile.values.get(&field.key) else { return false };
        if matches!(field.kind, FieldKind::Text { .. }) {
            let Value::Text(value) = current else { return false };
            self.view.address = value.clone();
            self.view.edit_field = Some(row - 1);
            self.open_sheet(Sheet::EditText, Focus::Keyboard(0));
            return true;
        }
        let next = match (&field.kind, current) {
            (FieldKind::Number { min, max, step, .. }, Value::Int(v)) => Value::Int(if right { v.saturating_add(*step).min(*max) } else { v.saturating_sub(*step).max(*min) }),
            (FieldKind::Toggle { .. }, Value::Bool(v)) => Value::Bool(!v),
            (FieldKind::Choice { options, .. }, Value::Text(v)) => {
                let Some(at) = options.iter().position(|o| &o.id == v) else { return false };
                let next = if right { (at + 1).min(options.len() - 1) } else { at.saturating_sub(1) };
                Value::Text(options[next].id.clone())
            }
            _ => return false,
        };
        if next != *current && field.kind.check(&next).is_ok() {
            if let Some(selector) = field.key.strip_prefix("view-") {
                let Value::Text(value) = next else { return false };
                let mut view = profile.view.clone();
                match selector {
                    "section" => { view.section = value; view.button = 1; view.step = 1; }
                    "profile" => { let Ok(n) = value.parse() else { return false }; view.profile = n; }
                    "sub" => { let Ok(n) = value.parse() else { return false }; view.sub = n; }
                    "side" => { let Ok(n) = value.parse() else { return false }; view.side = n; }
                    "button" => { let Ok(n) = value.parse() else { return false }; view.button = n; view.step = 1; }
                    "step" => { let Ok(n) = value.parse() else { return false }; view.step = n; }
                    "action" => { let Ok(n) = value.parse() else { return false }; view.action = n; }
                    _ => return false,
                }
                self.leadjoy.view = view;
                self.read_leadjoy(out);
            } else {
                let change = ControllerChange { setting: field.key.clone().into(), value: serde_json::json!(next) };
                self.leadjoy.pending = true;
                out.push(Command::Leadjoy { view: profile.view.clone(), change: Some(change) });
            }
        }
        true
    }
    fn delete_leadjoy(&mut self, out: &mut Vec<Command>) {
        if self.leadjoy.pending || self.leadjoy.error.is_some() { return; }
        let LeadjoyState::Ready(profile) = &self.leadjoy.state else { return };
        let Some(field) = self.view.edit_field.and_then(|i| profile.schema.fields().nth(i)).filter(|field| field.key.starts_with("delete-")) else { return };
        out.push(Command::Leadjoy { view: profile.view.clone(), change: Some(ControllerChange { setting: field.key.clone().into(), value: serde_json::Value::Null }) });
        self.leadjoy.pending = true;
    }
    fn adjust(&mut self, right: bool, out: &mut Vec<Command>) -> bool {
        if let Focus::EngineField(i) = self.view.focus {
            self.change_engine_field(i, right, out);
            return true;
        }
        if let Focus::Keyboard(_) = self.view.focus {
            return false;
        }
        let Focus::Setting(row) = self.view.focus else {
            return false;
        };
        if self.view.page == Page::GameSir {
            return self.adjust_gamesir(row, right, out);
        }
        if self.view.page == Page::Leadjoy { return self.adjust_leadjoy(row, right, out); }
        if self.view.page == Page::Integration {
            let Some(id) = self.view.selected_engine.clone() else {
                return false;
            };
            match row {
                0 => {
                    let enabled = !self.enabled(&id);
                    self.state.engines.entry(id.clone()).or_default().enabled = enabled;
                    out.push(Command::Enable { engine: id, on: enabled });
                    out.push(Command::SaveState);
                    self.rebuild_catalog();
                }
                1 => {
                    if self.enabled(&id) {
                        self.begin_auth(id, None, out);
                    }
                }
                2 => {
                    self.view.address.clear();
                    self.open_sheet(Sheet::AddHost, Focus::Keyboard(0));
                }
                _ => return false,
            }
            return true;
        }
        let step = |index: usize, len: usize| if right { (index + 1).min(len - 1) } else { index.saturating_sub(1) };
        if self.view.page == Page::Lab {
            let nudge = |v: u8, max: u8| if right { (v + 1).min(max) } else { v.saturating_sub(1) };
            let curves = [Curve::Linear, Curve::Precise, Curve::Quick];
            match row {
                0 => self.prefs.deadzone = nudge(self.prefs.deadzone, DEADZONE_MAX),
                1 => self.prefs.anti_deadzone = nudge(self.prefs.anti_deadzone, ANTI_DEADZONE_MAX),
                2 => self.prefs.stick_curve = curves[step(curves.iter().position(|c| *c == self.prefs.stick_curve).unwrap_or(0), curves.len())],
                _ => return false,
            }
            self.save_prefs(out);
            self.push_common(out);
            return true;
        }
        match self.view.section {
            Section::Appearance => match row {
                0 => return false,
                1 => self.prefs.light = !self.prefs.light,
                2 => self.prefs.hero = Density::ALL[step(self.prefs.hero.index(), 3)],
                3 => self.prefs.covers = Density::ALL[step(self.prefs.covers.index(), 3)],
                4 => self.prefs.text_percent = [100, 150, 200][step(usize::from((self.prefs.text_percent - 100) / 50), 3)],
                5 => self.prefs.reduce_motion = !self.prefs.reduce_motion,
                _ => return false,
            },
            Section::Streaming => match row {
                0 => {
                    let heights = [720, 1080, 1440, 2160];
                    let i = step(heights.iter().position(|h| *h == self.prefs.height).unwrap_or(3), 4);
                    self.prefs.height = heights[i];
                    self.prefs.width = [1280, 1920, 2560, 3840][i];
                }
                1 => {
                    self.prefs.fps = if right { 120 } else { 60 };
                    if self.prefs.fps > 60 && self.prefs.codec == Some(Codec::Av1) {
                        self.prefs.codec = None;
                    }
                }
                2 => self.prefs.hdr = !self.prefs.hdr,
                3 => self.prefs.bitrate_mbps = if right { (self.prefs.bitrate_mbps + 5).min(150) } else { self.prefs.bitrate_mbps.saturating_sub(5).max(5) },
                4 => {
                    let mut codecs = vec![None, Some(Codec::H265)];
                    if self.prefs.fps <= 60 {
                        codecs.push(Some(Codec::Av1));
                    }
                    codecs.push(Some(Codec::H264));
                    self.prefs.codec = codecs[step(codecs.iter().position(|c| *c == self.prefs.codec).unwrap_or(0), codecs.len())];
                }
                5 => {
                    let mut ids = vec![None];
                    ids.extend(self.integrations.iter().filter(|e| self.enabled(&e.manifest.id)).map(|e| Some(e.manifest.id.clone())));
                    let at = ids.iter().position(|id| *id == self.state.profile().user.preferred_engine).unwrap_or(0);
                    self.state.profile_mut().user.preferred_engine = ids[step(at, ids.len())].clone();
                }
                6 => {
                    self.set_stats(StatsMode::ALL[step(self.prefs.stats.mode.index(), 3)], out);
                    return true;
                }
                _ => return false,
            },
            _ => return false,
        }
        self.save_prefs(out);
        if self.view.section == Section::Streaming {
            self.push_common(out);
        }
        true
    }
    fn save_prefs(&mut self, out: &mut Vec<Command>) {
        self.state.profile_mut().ui = self.prefs.json();
        out.push(Command::SaveState);
    }
    fn stats_request(&self, out: &mut Vec<Command>) {
        if let Some(s) = self.session.as_ref().filter(|s| *s.phase() == Phase::Running) {
            let pad_timing = self.hud_visible();
            Self::send(out, &s.source.engine, Request::Stats { session: s.id, on: self.foreground && (pad_timing || self.view.page == Page::Overlay), pad_timing });
        }
    }
    fn set_stats(&mut self, mode: StatsMode, out: &mut Vec<Command>) {
        if mode == self.prefs.stats.mode { return; }
        let unix_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_millis() as u64);
        self.prefs.stats = Overlay { mode, changed: unix_ms.max(self.prefs.stats.changed.saturating_add(1)) };
        if !mode.on() { self.telemetry = Telemetry::default(); }
        self.save_prefs(out);
        self.stats_request(out);
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
            ("bitrate-mbps".into(), Value::Int(i64::from(self.prefs.bitrate_mbps))),
            ("codec".into(), Value::Text(self.prefs.codec.map_or("auto", Codec::as_str).into())),
            ("deadzone".into(), Value::Int(i64::from(self.prefs.deadzone))),
            ("anti-deadzone".into(), Value::Int(i64::from(self.prefs.anti_deadzone))),
            ("stick-curve".into(), Value::Text(self.prefs.stick_curve.name().into())),
        ])
    }
    fn change_engine_field(&mut self, index: usize, right: bool, out: &mut Vec<Command>) {
        let Some(id) = self.view.selected_engine.clone() else {
            return;
        };
        let Some(field) = self.integration(&id).and_then(|e| e.schema.fields().nth(index)).cloned() else {
            return;
        };
        if field.kind == FieldKind::Action {
            Self::send(out, &id, Request::Action { key: field.key });
            return;
        }
        let saved = self.state.engines.entry(id.clone()).or_default();
        let current = saved.settings.get(&field.key).cloned().or_else(|| field.kind.default());
        if let FieldKind::Text { .. } = field.kind {
            self.view.address = if let Some(Value::Text(value)) = current { value } else { String::new() };
            self.view.edit_field = Some(index);
            self.open_sheet(Sheet::EditText, Focus::Keyboard(0));
            return;
        }
        let next = match (&field.kind, current) {
            (FieldKind::Toggle { .. }, Some(Value::Bool(v))) => Some(Value::Bool(!v)),
            (FieldKind::Choice { options, .. }, Some(Value::Text(v))) if !options.is_empty() => {
                let i = options.iter().position(|o| o.id == v).unwrap_or(0);
                let j = if right { (i + 1).min(options.len() - 1) } else { i.saturating_sub(1) };
                Some(Value::Text(options[j].id.clone()))
            }
            (FieldKind::Number { min, max, step, .. }, Some(Value::Int(v))) => {
                Some(Value::Int(if right { v.saturating_add(*step).min(*max) } else { v.saturating_sub(*step).max(*min) }))
            }
            _ => None,
        };
        if let Some(value) = next {
            saved.settings.insert(field.key, value);
            Self::send(out, &id, Request::Settings { values: saved.settings.clone() });
            out.push(Command::SaveState);
        }
    }
    fn open_chooser(&mut self) {
        if let Some(id) = self.view.selected {
            let desired = self.play_target(id);
            self.view.chooser_selected = desired.clone();
            let i = self.game(id).and_then(|g| g.sources.iter().position(|s| Some(s.key()) == desired)).unwrap_or(0);
            self.view.remember = self.state.profile().user.pinned.contains_key(&id);
            self.open_sheet(Sheet::Chooser, Focus::SheetRow(i));
        }
    }
    fn play(&mut self, out: &mut Vec<Command>) {
        let Some(game) = self.view.selected else {
            return;
        };
        if let Some(key) = self.view.chooser_selected.clone() {
            if self.readiness(&key).can_attempt() {
                self.start(game, key, false, out);
            } else {
                self.open_chooser();
            }
            return;
        }
        match self.resolution(game) {
            Resolution::Launch { key, .. } => self.start(game, key, false, out),
            Resolution::Choose { .. } => self.open_chooser(),
            Resolution::Nothing => {
                self.view.section = Section::Integrations;
                self.show_page(Page::Settings, Focus::Integration(0));
            }
        }
    }
    fn start(&mut self, game: GameId, source: SourceKey, remember: bool, out: &mut Vec<Command>) {
        if self.session_active() {
            return;
        }
        let presentation = self.integration(&source.engine).and_then(|e| e.descriptor.as_ref()).map_or(Presentation::Feeder, |d| d.capabilities.presentation);
        let (session, effects) = Session::start(SessionId(self.next_session), game, source, presentation, remember, self.now_ms);
        self.next_session = self.next_session.wrapping_add(1);
        self.session_start = self.unix_s;
        self.session = Some(session);
        self.view.selected = Some(game);
        self.view.notice = None;
        self.view.stream = None;
        self.view.stats = Stats::default();
        self.telemetry = Telemetry::default();
        self.view.page = Page::Launch;
        self.view.sheet = None;
        self.view.focus = Focus::None;
        self.session_effects(effects, out);
    }
    fn session_input(&mut self, input: session::Input, out: &mut Vec<Command>) {
        if let Some(session) = &mut self.session {
            let effects = session.step(self.now_ms, input);
            self.session_effects(effects, out);
        }
    }
    fn checkpoint(&self, seconds: u32) -> Option<OpenSession> {
        let s = self.session.as_ref()?;
        Some(OpenSession { profile: self.state.active.clone(), played: Played { game: s.game, source: s.source.clone(), start: self.session_start, seconds } })
    }
    fn session_effects(&mut self, effects: Vec<session::Effect>, out: &mut Vec<Command>) {
        for effect in effects {
            let Some(s) = &self.session else {
                break;
            };
            match effect {
                session::Effect::Launch => {
                    Self::send(out, &s.source.engine, Request::Launch { session: s.id, target: s.source.target.clone(), game: s.source.id.clone() })
                }
                session::Effect::Stop { end_game } => {
                    Self::send(out, &s.source.engine, Request::Stop { session: s.id, end_game });
                    self.view.page = Page::Launch;
                    self.view.sheet = None;
                }
                session::Effect::LowLatency(on) => out.push(Command::LowLatency(on)),
                session::Effect::Started { pin } => {
                    self.state.profile_mut().user.started(s.game, &s.source, pin);
                    self.session_start = self.unix_s;
                    self.view.page = Page::Running;
                    self.view.sheet = None;
                    self.view.auth = None;
                    out.push(Command::SaveState);
                    self.stats_request(out);
                    if let Some(checkpoint) = self.checkpoint(0) {
                        out.push(Command::Checkpoint(checkpoint));
                    }
                }
                session::Effect::Checkpoint { seconds } => {
                    if let Some(checkpoint) = self.checkpoint(seconds) {
                        out.push(Command::Checkpoint(checkpoint));
                    }
                }
                session::Effect::EndEngine => out.push(Command::KillEngine(s.source.engine.clone())),
                session::Effect::Finished { played, outcome } => {
                    if let Some(checkpoint) = played.and_then(|seconds| self.checkpoint(seconds)) {
                        self.state.record(checkpoint);
                        out.push(Command::SaveState);
                    }
                    self.view.page = Page::Hub;
                    self.view.sheet = None;
                    self.view.focus = Focus::HubAction(0);
                    self.view.hub_tab = 0;
                    self.view.notice = match outcome {
                        session::Outcome::Failed { message, .. } => Some(message),
                        _ => None,
                    };
                    out.push(Command::ClearCheckpoint);
                    self.rebuild_rows();
                    if self.closing {
                        out.push(Command::Exit);
                    }
                }
            }
        }
    }
    fn open_overlay(&mut self, out: &mut Vec<Command>) {
        if let Some(s) = self.session.as_ref().filter(|s| *s.phase() == Phase::Running) {
            self.view.page = Page::Overlay;
            self.view.focus = Focus::Overlay(0);
            self.view.sheet = None;
            Self::send(out, &s.source.engine, Request::Route { session: s.id, input: ipc::InputRoute::App });
            self.stats_request(out);
        }
    }
    fn resume(&mut self, out: &mut Vec<Command>) {
        if let Some(s) = &self.session {
            self.view.page = Page::Running;
            self.view.sheet = None;
            self.view.focus = Focus::None;
            Self::send(out, &s.source.engine, Request::Route { session: s.id, input: ipc::InputRoute::Game });
            self.stats_request(out);
        }
    }
    pub fn can_end_game(&self) -> bool {
        self.session
            .as_ref()
            .and_then(|s| self.integration(&s.source.engine))
            .and_then(|e| e.descriptor.as_ref())
            .is_some_and(|d| d.capabilities.has(Feature::EndGame))
    }
    fn go_back(&mut self, out: &mut Vec<Command>) {
        if self.view.sheet.is_some() {
            if self.view.sheet == Some(Sheet::Auth) {
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
            Page::Settings if self.overlay_return => {
                self.overlay_return = false;
                self.view.page = Page::Overlay;
                self.view.focus = Focus::Overlay(1);
            }
            Page::Settings if matches!(self.view.focus, Focus::Setting(_) | Focus::Integration(_) | Focus::Theme(_)) => {
                self.view.focus = Focus::Rail(self.view.section.index());
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
    fn begin_auth(&mut self, engine: EngineId, target: Option<TargetId>, out: &mut Vec<Command>) {
        let method = self.integration(&engine).and_then(|e| e.descriptor.as_ref()).and_then(|d| d.capabilities.auth.first()).copied();
        self.view.auth = Some(LinkFlow::new(engine.clone(), target.clone(), self.now_ms));
        self.open_sheet(Sheet::Auth, Focus::SheetAction(0));
        if !self.enabled(&engine) {
            self.state.engines.entry(engine.clone()).or_default().enabled = true;
            out.push(Command::Enable { engine, on: true });
            out.push(Command::SaveState);
        } else if let Some(method) = method {
            Self::send(out, &engine, Request::Auth { target, method });
        } else {
            self.view.notice = Some("This integration does not need a sign-in.".into());
            self.close_sheet();
        }
    }
    fn auth_action(&mut self, action: usize, out: &mut Vec<Command>) {
        let Some(a) = &self.view.auth else {
            return;
        };
        if a.no_pcs {
            let engine = a.engine.clone();
            Self::send(out, &engine, Request::AuthCancel);
            self.view.auth = None;
            self.close_sheet();
            if action == 1 {
                self.view.selected_engine = Some(engine);
                self.view.address.clear();
                self.open_sheet(Sheet::AddHost, Focus::Keyboard(0));
            } else {
                Self::send(out, &engine, Request::Targets);
                self.begin_auth(engine, None, out);
            }
            return;
        }
        match &a.state {
            Some(AuthState::EnterPin { digits }) if a.pin.len() == usize::from(*digits) => Self::send(out, &a.engine, Request::AuthPin { pin: a.pin.clone() }),
            Some(AuthState::Failed { .. }) => {
                let id = a.engine.clone();
                let target = a.target.clone();
                self.begin_auth(id, target, out);
            }
            Some(AuthState::EnterPin { .. }) => {}
            _ => self.go_back(out),
        }
    }
    fn keyboard(&mut self, i: usize, out: &mut Vec<Command>) {
        let numeric = self.view.sheet == Some(Sheet::Auth);
        if !numeric && i == 43 {
            self.view.keyboard_lower = !self.view.keyboard_lower;
            return;
        }
        let chars = if numeric {
            "1234567890"
        } else if self.view.keyboard_lower {
            "abcdefghijklmnopqrstuvwxyz0123456789.-:/"
        } else {
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-:/"
        };
        let limit = if numeric {
            self.view
                .auth
                .as_ref()
                .and_then(|a| match a.state {
                    Some(AuthState::EnterPin { digits }) => Some(usize::from(digits)),
                    _ => None,
                })
                .unwrap_or(12)
        } else if self.view.sheet.is_some() {
            255
        } else {
            128
        };
        let text = match self.view.sheet {
            Some(Sheet::Auth) => self.view.auth.as_mut().map(|a| &mut a.pin),
            Some(Sheet::AddHost | Sheet::EditText) => Some(&mut self.view.address),
            _ => Some(&mut self.view.query),
        };
        if let Some(text) = text {
            if let Some(c) = chars.chars().nth(i) {
                if text.chars().count() < limit {
                    text.push(c);
                }
            } else if i == chars.len() {
                text.pop();
            } else if i == chars.len() + 1 && !numeric {
                if text.chars().count() < limit {
                    text.push(' ');
                }
            } else if numeric {
                self.auth_action(0, out);
            } else if self.view.sheet == Some(Sheet::EditText) {
                self.save_text(out);
            } else if self.view.sheet == Some(Sheet::AddHost) {
                if let Some(id) = &self.view.selected_engine {
                    Self::send(out, id, Request::AddTarget { address: self.view.address.clone() });
                }
                self.close_sheet();
            } else {
                self.view.focus = Focus::Grid(0);
            }
        }
        if self.view.sheet.is_none() {
            self.rebuild_filter();
        }
    }
    fn save_text(&mut self, out: &mut Vec<Command>) {
        if self.view.page == Page::Leadjoy {
            if self.leadjoy.pending || self.leadjoy.error.is_some() { return; }
            let LeadjoyState::Ready(profile) = &self.leadjoy.state else { return };
            let Some(field) = self.view.edit_field.and_then(|i| profile.schema.fields().nth(i)) else { return };
            let value = Value::Text(self.view.address.clone());
            if !matches!(field.kind, FieldKind::Text { .. }) || field.kind.check(&value).is_err() {
                self.view.notice = Some("The name is too long. Shorten it before saving.".into());
                return;
            }
            out.push(Command::Leadjoy { view: profile.view.clone(), change: Some(ControllerChange { setting: field.key.clone().into(), value: serde_json::json!(value) }) });
            self.leadjoy.pending = true;
            self.close_sheet();
            return;
        }
        let Some(id) = self.view.selected_engine.clone() else {
            return;
        };
        let Some(index) = self.view.edit_field else {
            return;
        };
        let Some(engine) = self.integration(&id) else {
            return;
        };
        let Some(field) = engine.schema.fields().nth(index) else {
            return;
        };
        let value = Value::Text(self.view.address.clone());
        if engine.schema.check(&field.key, &value).is_err() {
            self.view.notice = Some(match field.kind {
                FieldKind::Text { max_chars, .. } => format!("Use at most {max_chars} characters."),
                _ => "The integration changed this field. Open it again.".into(),
            });
            return;
        }
        let key = field.key.clone();
        let saved = self.state.engines.entry(id.clone()).or_default();
        saved.settings.insert(key, value);
        Self::send(out, &id, Request::Settings { values: saved.settings.clone() });
        out.push(Command::SaveState);
        self.close_sheet();
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
                    self.rebuild_catalog();
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
                    self.open_sheet(Sheet::Auth, Focus::SheetAction(0));
                }
                if matches!(state, AuthState::Done { .. }) {
                    // The Targets answer starts the library traversal; a second one here overlapped it, and the GeForce NOW
                    // engine rejected the shared first cursor as repeated.
                    Self::send(out, &id, Request::Targets);
                    self.view.auth = None;
                    self.close_sheet();
                } else if let Some(a) = &mut self.view.auth {
                    a.expires_ms = match state {
                        AuthState::DeviceCode { expires_s, .. } => Some(self.now_ms + u64::from(expires_s) * 1000),
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
            ipc::Event::Key { key, down } => {
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
                if self.engine_in_view(&id) {
                    self.view.notice = Some(message.clone());
                }
                if let Some(e) = self.integrations.iter_mut().find(|e| e.manifest.id == id) {
                    e.error = Some(message);
                }
            }
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
                if let Some(details) = self.details.get(&id) {
                    wanted.extend(details.media.iter().map(|m| m.image.clone()));
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
                            HomeItem::Genre(name) => {
                                ids.extend(self.games.iter().filter(|g| self.details.get(&g.id).is_some_and(|d| d.genres.contains(name))).take(3).map(|g| g.id))
                            }
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
        if let Some(sheet) = self.view.sheet {
            match sheet {
                Sheet::Chooser => {
                    if let Some(g) = self.selected_game() {
                        rows.extend((0..g.sources.len()).map(|i| vec![Focus::SheetRow(i)]));
                    }
                    rows.push(vec![Focus::Remember]);
                    rows.push(vec![Focus::SheetAction(0)]);
                }
                Sheet::Sort => rows.extend((0..4).map(|i| vec![Focus::SheetRow(i)])),
                Sheet::Genre => rows.extend((0..=self.genres().len()).map(|i| vec![Focus::SheetRow(i)])),
                Sheet::Theme => {
                    let count = lumatv_tokens::themes::game_options().len();
                    let cols = if self.prefs.text_percent > 100 { 3 } else { 4 };
                    for i in (0..count).step_by(cols) {
                        rows.push((i..(i + cols).min(count)).map(Focus::Theme).collect());
                    }
                }
                Sheet::EndSession | Sheet::Delete => rows.push(vec![Focus::SheetAction(0), Focus::SheetAction(1)]),
                Sheet::Auth if self.view.auth.as_ref().is_some_and(|a| matches!(a.state, Some(AuthState::EnterPin { .. }))) => {
                    for i in (0..12).step_by(6) {
                        rows.push((i..i + 6).map(Focus::Keyboard).collect());
                    }
                    rows.push(vec![Focus::SheetAction(0)]);
                }
                Sheet::AddHost | Sheet::EditText => {
                    for i in (0..44).step_by(10) {
                        rows.push((i..(i + 10).min(44)).map(Focus::Keyboard).collect());
                    }
                    rows.push(vec![Focus::SheetAction(0)]);
                }
                Sheet::Auth if self.view.auth.as_ref().is_some_and(|a| a.no_pcs) => {
                    let manual = self.view.auth.as_ref().and_then(|a| self.integration(&a.engine)).and_then(|e| e.descriptor.as_ref()).is_some_and(|d| d.capabilities.has(Feature::ManualHost));
                    rows.push(if manual { vec![Focus::SheetAction(0), Focus::SheetAction(1)] } else { vec![Focus::SheetAction(0)] });
                }
                Sheet::Auth => rows.push(vec![Focus::SheetAction(0)]),
            }
        } else {
            match self.view.page {
                Page::Home | Page::Library | Page::Search | Page::Settings | Page::Integration | Page::Lab | Page::GameSir | Page::Leadjoy => rows.push((0..4).map(Focus::Nav).collect()),
                _ => {}
            }
            match self.view.page {
                Page::Home if self.games.is_empty() => rows.push((0..self.integrations.len()).map(Focus::FirstRun).collect()),
                Page::Home => {
                    if self.prefs.text_percent > 100 { rows.extend([vec![Focus::Hero(0)], vec![Focus::Hero(1)]]); } else { rows.push(vec![Focus::Hero(0), Focus::Hero(1)]); }
                    for (r, row) in self.home_rows.iter().enumerate() {
                        rows.push((0..row.items.len()).map(|i| Focus::Home(r, i)).collect());
                    }
                }
                Page::Library | Page::Search => {
                    if self.view.page == Page::Library {
                        rows.push((0..self.integrations.iter().filter(|e| self.enabled(&e.manifest.id)).count() + 4).map(Focus::Filter).collect());
                    } else {
                        rows.push(vec![Focus::SearchField]);
                        for i in (0..44).step_by(10) {
                            rows.push((i..(i + 10).min(44)).map(Focus::Keyboard).collect());
                        }
                    }
                    let cols = self.prefs.covers.columns();
                    for i in (0..self.filtered.len()).step_by(cols) {
                        rows.push((i..(i + cols).min(self.filtered.len())).map(Focus::Grid).collect());
                    }
                }
                Page::Hub => {
                    rows.push((0..3).map(Focus::HubTop).collect());
                    if self.view.hub_tab == 0 {
                        if self.prefs.text_percent > 100 { rows.extend([vec![Focus::HubAction(0)], vec![Focus::HubAction(1)]]); } else { rows.push(vec![Focus::HubAction(0), Focus::HubAction(1)]); }
                    }
                    rows.push((0..3).map(Focus::Tab).collect());
                    if self.view.hub_tab == 1 {
                        rows.extend((0..=self.selected_game().map_or(0, |g| g.sources.len())).map(|i| vec![Focus::Way(i)]));
                    }
                    if self.view.hub_tab == 2 {
                        if let Some(details) = self.view.selected.and_then(|id| self.details.get(&id)) {
                            for video in [true, false] {
                                let row = details
                                    .media
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, m)| m.video.is_some() == video)
                                    .map(|(i, _)| Focus::Media(i))
                                    .collect::<Vec<_>>();
                                if !row.is_empty() {
                                    rows.push(row);
                                }
                            }
                        }
                    }
                }
                Page::Settings => {
                    if self.view.section == Section::Appearance {
                        rows.push((1..=if self.prefs.text_percent > 100 { 2 } else { 4 }).map(Focus::Theme).collect());
                    }
                    let count = match self.view.section {
                        Section::Appearance => 6,
                        Section::Streaming => 7,
                        Section::Integrations => self.integrations.len(),
                        Section::Controllers => 4,
                        Section::About => 3,
                    };
                    for i in 0..count.max(5) {
                        let mut row = Vec::new();
                        if i < 5 {
                            row.push(Focus::Rail(i));
                        }
                        if i < count {
                            row.push(if self.view.section == Section::Integrations { Focus::Integration(i) } else { Focus::Setting(i) });
                        }
                        rows.push(row);
                    }
                }
                Page::Integration => {
                    rows.push(vec![Focus::Setting(0)]);
                    if let Some(e) = self.view.selected_engine.as_ref().and_then(|id| self.integration(id)).filter(|e| self.enabled(&e.manifest.id)) {
                        if e.descriptor.as_ref().is_some_and(|d| !d.capabilities.auth.is_empty()) {
                            rows.push(vec![Focus::Setting(1)]);
                        }
                        if e.descriptor.as_ref().is_some_and(|d| d.capabilities.has(Feature::ManualHost)) {
                            rows.push(vec![Focus::Setting(2)]);
                        }
                        rows.extend((0..e.targets.len()).map(|i| vec![Focus::Target(i)]));
                        rows.extend(e.schema.fields().enumerate().filter(|(_, f)| f.kind != FieldKind::Note).map(|(i, _)| vec![Focus::EngineField(i)]));
                    }
                }
                Page::Overlay => rows.push((0..4).map(Focus::Overlay).collect()),
                Page::Media => rows.push(vec![Focus::HubTop(0)]),
                Page::Lab => {
                    if !self.session_active() {
                        if self.prefs.text_percent > 100 { rows.extend([vec![Focus::Lab(0), Focus::Lab(1)], vec![Focus::Lab(2), Focus::Lab(3)]]); } else { rows.push((0..4).map(Focus::Lab).collect()); }
                        if self.lab.pad().is_some() {
                            rows.extend((5..8).map(|i| vec![Focus::Lab(i)]));
                        }
                    } else if self.round_trip_engine().is_some() && !matches!(self.lab.trip, Trip::Unsupported(_)) {
                        rows.push(vec![Focus::Lab(4)]);
                        if matches!(self.lab.trip, Trip::Answer(..)) {
                            rows.extend((8..13).map(|i| vec![Focus::Lab(i)]));
                        }
                    }
                    rows.extend((0..3).map(|i| vec![Focus::Setting(i)]));
                }
                Page::GameSir => {
                    let count = if matches!(self.gamesir.state, GameSirState::Ready(_)) { 8 } else { 1 };
                    rows.extend((0..count).map(|i| vec![Focus::Setting(i)]));
                }
                Page::Leadjoy => {
                    let count = if let LeadjoyState::Ready(profile) = &self.leadjoy.state { profile.schema.fields().count() + 1 } else { 1 };
                    rows.extend((0..count).map(|i| vec![Focus::Setting(i)]));
                }
                Page::Running | Page::Launch => {}
            }
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
            let count = self.navigation.iter().filter(|n| matches!(n.id, Focus::Setting(_) | Focus::Integration(_))).count();
            let pane = |i| if self.view.section == Section::Integrations { Focus::Integration(i) } else { Focus::Setting(i) };
            let section = self.view.section.index();
            for n in &mut self.navigation {
                match n.id {
                    Focus::Rail(i) => {
                        n.left = None;
                        n.right = if self.view.section == Section::Appearance {
                            Some(Focus::Theme(1))
                        } else if count > 0 {
                            Some(pane(0))
                        } else {
                            None
                        };
                        n.up = Some(if i == 0 { Focus::Nav(3) } else { Focus::Rail(i - 1) });
                        n.down = (i + 1 < 5).then_some(Focus::Rail(i + 1));
                    }
                    Focus::Setting(i) | Focus::Integration(i) => {
                        n.left = Some(Focus::Rail(section));
                        n.right = None;
                        n.up = Some(if i > 0 {
                            pane(i - 1)
                        } else if self.view.section == Section::Appearance {
                            Focus::Theme(1)
                        } else {
                            Focus::Nav(3)
                        });
                        n.down = (i + 1 < count).then_some(pane(i + 1));
                    }
                    Focus::Theme(_) => {
                        n.up = Some(Focus::Nav(3));
                        n.down = Some(Focus::Setting(0));
                    }
                    _ => {}
                }
            }
        }
        if !self.navigation.iter().any(|n| n.id == self.view.focus) {
            self.view.focus = self.navigation.first().map_or(Focus::None, |n| n.id);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    fn key(m: &mut Model, key: Key) -> Vec<Command> {
        m.step(Input::Key { key, down: true });
        m.step(Input::Key { key, down: false })
    }
    /// Sunshine's pairing sheet once its search has found no PC.
    pub(crate) fn no_pc_found() -> Model {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gamestream", "Sunshine", EngineKind::Host)]);
        describe(&mut m, "gamestream", vec![lumagame_core::engine::AuthMethod::ShowPin], &[Feature::ManualHost]);
        m.begin_auth(EngineId::new("gamestream").unwrap(), None, &mut Vec::new());
        m.step(Input::Tick { now_ms: PC_SEARCH_MS, unix_s: 0 });
        m
    }
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
    #[test]
    fn stream_settings_limit_av1_and_bitrate_without_changing_other_fields() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        m.view.page = Page::Settings;
        m.view.section = Section::Streaming;
        m.prefs.codec = Some(Codec::Av1);
        m.view.focus = Focus::Setting(1);
        key(&mut m, Key::Right);
        assert_eq!(m.prefs.fps, 120);
        assert_eq!(m.prefs.codec, None);
        m.view.focus = Focus::Setting(3);
        m.prefs.bitrate_mbps = 150;
        key(&mut m, Key::Right);
        assert_eq!(m.prefs.bitrate_mbps, 150);
    }
    #[test]
    fn sheets_return_focus_and_destructive_confirmation_starts_safe() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        m.view.page = Page::Overlay;
        m.view.focus = Focus::Overlay(3);
        m.rebuild_navigation();
        key(&mut m, Key::Ok);
        assert_eq!(m.view.sheet, Some(Sheet::EndSession));
        assert_eq!(m.view.focus, Focus::SheetAction(0));
        key(&mut m, Key::Back);
        assert_eq!(m.view.focus, Focus::Overlay(3));
    }
    #[test]
    fn no_history_means_no_empty_home_rows_and_idle_tick_does_not_relayout() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        assert!(m.home_rows.is_empty());
        let r = m.revision();
        assert!(m.step(Input::Tick { now_ms: 1000, unix_s: 100 }).is_empty());
        assert_eq!(m.revision(), r);
    }
    #[test]
    fn sign_in_starts_one_library_traversal_through_targets() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        let engine = EngineId::new("gfn").unwrap();
        let done = ipc::Event::Auth { state: AuthState::Done { target: TargetId::new("main").unwrap() } };
        let commands = m.step(Input::Engine { engine, message: FromEngine { re: None, event: done } });
        let sent: Vec<_> = commands.iter().filter_map(|c| if let Command::Send { request, .. } = c { Some(request) } else { None }).collect();
        assert!(matches!(sent.as_slice(), [Request::Targets]));
    }
    #[test]
    fn repeated_targets_leave_the_running_library_traversal_alone() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        let engine = EngineId::new("gfn").unwrap();
        let target = ipc::Target { id: TargetId::new("main").unwrap(), name: "nphil".into(), detail: None, link: ipc::Link::Linked, reach: ipc::Reach::Online };
        let deliver = |m: &mut Model, event| {
            m.step(Input::Engine { engine: engine.clone(), message: FromEngine { re: None, event } })
                .into_iter().filter(|c| matches!(c, Command::Send { request: Request::Library { .. }, .. })).count()
        };
        assert_eq!(deliver(&mut m, ipc::Event::Targets { targets: vec![target.clone()] }), 1);
        assert_eq!(deliver(&mut m, ipc::Event::Targets { targets: vec![target.clone()] }), 0);
        assert_eq!(deliver(&mut m, ipc::Event::Library { target: target.id.clone(), entries: vec![], next: None }), 0);
        assert_eq!(deliver(&mut m, ipc::Event::Targets { targets: vec![target] }), 1);
    }
    fn manifest(id: &str, name: &str, kind: EngineKind) -> Manifest {
        Manifest { id: EngineId::new(id).unwrap(), name: name.into(), summary: String::new(), kind, protocol: 1, exec: None, args: Vec::new(), license: "MIT".into(), source: None }
    }
    fn describe(m: &mut Model, id: &str, auth: Vec<lumagame_core::engine::AuthMethod>, features: &[Feature]) {
        let id = EngineId::new(id).unwrap();
        m.state.engines.entry(id.clone()).or_default().enabled = true;
        let e = m.integrations.iter_mut().find(|e| e.manifest.id == id).unwrap();
        e.descriptor = Some(Descriptor {
            id,
            version: "1".into(),
            capabilities: lumagame_core::engine::Capabilities {
                kind: e.manifest.kind,
                targets: lumagame_core::engine::TargetKind::Host,
                auth,
                presentation: Presentation::Feeder,
                features: features.iter().copied().collect(),
                video: None,
            },
        });
    }
    fn sent(commands: &[Command]) -> Vec<&Request> {
        commands.iter().filter_map(|c| if let Command::Send { request, .. } = c { Some(request) } else { None }).collect()
    }
    #[test]
    fn integrations_of_one_kind_sit_under_one_heading() {
        let m = Model::new(
            State::default(),
            Listing::default(),
            vec![manifest("gamestream", "Sunshine", EngineKind::Host), manifest("gfn", "GeForce NOW", EngineKind::Cloud), manifest("punktfunk", "Punktfunk", EngineKind::Host)],
        );
        let order: Vec<_> = m.integrations.iter().map(|e| e.manifest.id.as_str()).collect();
        assert_eq!(order, ["gfn", "gamestream", "punktfunk"]);
    }
    #[test]
    fn play_time_reads_in_minutes_under_an_hour() {
        assert_eq!(play_time(30), "Under a minute");
        assert_eq!(play_time(600), "10 min");
        assert_eq!(play_time(3600), "1 h");
        assert_eq!(play_time(5400), "1 h 30 min");
    }
    #[test]
    fn ending_a_cloud_session_names_the_service_not_a_pc() {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud), manifest("gamestream", "Sunshine", EngineKind::Host)]);
        describe(&mut m, "gfn", vec![], &[Feature::EndGame]);
        describe(&mut m, "gamestream", vec![], &[Feature::EndGame]);
        let key = |engine: &str| SourceKey { engine: EngineId::new(engine).unwrap(), target: TargetId::new("main").unwrap(), id: lumagame_core::ids::ProviderGameId::new("1").unwrap() };
        m.session = Some(Session::start(SessionId(1), GameId(1), key("gfn"), Presentation::Feeder, false, 0).0);
        assert!(!m.wants_pads(), "the menu lets go of the pads as soon as a launch begins");
        assert_eq!(m.end_copy().1, "The game will close on GeForce NOW. Unsaved progress may be lost.");
        m.session = Some(Session::start(SessionId(2), GameId(1), key("gamestream"), Presentation::Feeder, false, 0).0);
        assert_eq!(m.end_copy().1, "The game will close on the PC. Unsaved progress may be lost.");
    }
    #[test]
    fn pairing_with_no_pc_found_offers_an_address_or_another_search() {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gamestream", "Sunshine", EngineKind::Host)]);
        describe(&mut m, "gamestream", vec![lumagame_core::engine::AuthMethod::ShowPin], &[Feature::ManualHost]);
        let id = EngineId::new("gamestream").unwrap();
        let mut out = Vec::new();
        m.begin_auth(id.clone(), None, &mut out);
        m.step(Input::Tick { now_ms: PC_SEARCH_MS - 1, unix_s: 0 });
        assert!(!m.view.auth.as_ref().unwrap().no_pcs);
        m.step(Input::Tick { now_ms: PC_SEARCH_MS, unix_s: 0 });
        assert!(m.view.auth.as_ref().unwrap().no_pcs);
        assert!(m.navigation.iter().any(|n| n.id == Focus::SheetAction(1)));

        m.view.focus = Focus::SheetAction(0);
        let retry = key(&mut m, Key::Ok);
        assert!(matches!(sent(&retry).as_slice(), [Request::AuthCancel, Request::Targets, Request::Auth { target: None, .. }]));
        let flow = m.view.auth.as_ref().unwrap();
        assert!(!flow.no_pcs && flow.since_ms == PC_SEARCH_MS, "a retry searches for the full time again");

        m.step(Input::Tick { now_ms: 2 * PC_SEARCH_MS, unix_s: 0 });
        m.view.focus = Focus::SheetAction(1);
        let manual = key(&mut m, Key::Ok);
        assert!(matches!(sent(&manual).as_slice(), [Request::AuthCancel]));
        assert_eq!((m.view.sheet, m.view.focus), (Some(Sheet::AddHost), Focus::Keyboard(0)));
        assert_eq!(m.view.selected_engine, Some(id.clone()));

        let cancelled = AuthState::Failed { reason: ipc::AuthFail::Cancelled, message: "Link cancelled.".into() };
        m.step(Input::Engine { engine: id, message: FromEngine { re: None, event: ipc::Event::Auth { state: cancelled } } });
        assert_eq!(m.view.sheet, Some(Sheet::AddHost), "the answer to the cancel leaves the address entry open");
    }

    mod lab_tests {
        use super::*;

        fn lab() -> Model {
            let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud)]);
            m.view.page = Page::Settings;
            m.view.section = Section::Controllers;
            m.view.focus = Focus::Setting(0);
            key(&mut m, Key::Ok);
            m
        }
        fn reports(m: &mut Model, now_ns: u64, frame: u64, reports: Vec<(u8, RawReport)>) {
            m.step(Input::Lab { now_ns, frame, reports, names: [Some("Cyclone 2".into()), None, None, None] });
        }
        fn report(kernel_ns: u64, buttons: u16) -> (u8, RawReport) {
            (0, RawReport { kernel_ns, read_ns: kernel_ns + 100, buttons, ..Default::default() })
        }
        fn session(m: &mut Model, features: &[Feature]) {
            describe(m, "gfn", vec![], features);
            m.session = Some(Session::start(SessionId(1), GameId(1), "gfn|main|1".parse().unwrap(), Presentation::Feeder, false, 0).0);
            m.rebuild_navigation();
        }

        #[test]
        fn controllers_open_the_lab_and_back_returns_to_the_same_row() {
            let mut m = lab();
            assert_eq!((m.view.page, m.view.focus, m.pad_mode()), (Page::Lab, Focus::Lab(0), PadMode::Lab));
            key(&mut m, Key::Back);
            assert_eq!((m.view.page, m.view.section, m.view.focus), (Page::Settings, Section::Controllers, Focus::Setting(0)));
            m.view.focus = Focus::Setting(1);
            assert!(key(&mut m, Key::Ok).contains(&Command::OpenControllerSettings));
        }

        #[test]
        fn capture_ignores_keys_and_remote_back_exits_on_release() {
            let mut m = lab();
            key(&mut m, Key::Ok);
            assert_eq!(m.pad_mode(), PadMode::Captured);
            let focus = m.view.focus;
            for k in [Key::Up, Key::Right, Key::Ok, Key::Guide] { key(&mut m, k); }
            assert_eq!(m.view.focus, focus);
            assert_eq!(m.lab.capture, Some(Capture::Buttons));
            m.step(Input::Key { key: Key::Back, down: true });
            assert_eq!(m.pad_mode(), PadMode::Captured);
            m.step(Input::Key { key: Key::Back, down: false });
            assert_eq!(m.pad_mode(), PadMode::Lab);
            m.view.sheet = Some(Sheet::Theme);
            assert_eq!(m.pad_mode(), PadMode::Menu);
            m.view.sheet = None;
            m.step(Input::Foreground(false));
            assert_eq!(m.pad_mode(), PadMode::Menu);
        }

        #[test]
        fn b_hold_exits_at_one_second_without_another_report() {
            let mut m = lab();
            key(&mut m, Key::Ok);
            reports(&mut m, 10, 1, vec![report(10, button::B)]);
            reports(&mut m, 10 + LEAVE_HOLD_NS - 1, 2, vec![]);
            assert_eq!(m.lab.capture, Some(Capture::Buttons));
            reports(&mut m, 10 + LEAVE_HOLD_NS, 3, vec![]);
            assert_eq!(m.lab.capture, None);
        }

        #[test]
        fn flash_uses_the_kernel_edge_and_survives_a_batched_release_for_one_frame() {
            let mut m = lab();
            m.view.focus = Focus::Lab(3);
            key(&mut m, Key::Ok);
            reports(&mut m, 20, 7, vec![report(10, button::A), report(15, 0)]);
            assert_eq!(m.flash_press(), Some((10, 7)));
            reports(&mut m, 21, 7, vec![]);
            assert_eq!(m.flash_press(), Some((10, 7)), "no buffer swap yet");
            reports(&mut m, 22, 8, vec![]);
            assert_eq!(m.flash_press(), None);
            reports(&mut m, 30, 9, vec![report(25, button::X)]);
            assert_eq!(m.flash_press(), Some((25, 9)));
            reports(&mut m, 40, 10, vec![report(35, 0)]);
            assert_eq!(m.flash_press(), None);
        }

        #[test]
        fn rumble_tests_target_connected_pads_and_the_selected_motor() {
            let mut m = lab();
            reports(&mut m, 1, 1, vec![]);
            m.view.focus = Focus::Lab(1);
            assert!(key(&mut m, Key::Ok).contains(&Command::Rumble { slot: 0, low: 0xc000, high: 0 }));
            m.view.focus = Focus::Lab(2);
            assert!(key(&mut m, Key::Ok).contains(&Command::Rumble { slot: 0, low: 0, high: 0xc000 }));
        }

        #[test]
        fn stick_choices_are_saved_and_sent_only_under_the_common_keys() {
            let mut m = lab();
            describe(&mut m, "gfn", vec![], &[]);
            m.integrations[0].schema = serde_json::from_value(serde_json::json!({"sections":[{"title":"Sticks","fields":[
                {"key":"deadzone","label":"Dead zone","type":"number","min":0,"max":20,"step":1,"default":8},
                {"key":"anti-deadzone","label":"Anti-deadzone","type":"number","min":0,"max":30,"step":1,"default":0},
                {"key":"stick-curve","label":"Curve","type":"choice","options":[{"id":"linear","label":"Linear"},{"id":"precise","label":"Precise"},{"id":"quick","label":"Quick"}],"default":"linear"}
            ]}]})).unwrap();
            for row in 0..3 {
                m.view.focus = Focus::Setting(row);
                let commands = m.step(Input::Key { key: Key::Right, down: true });
                assert!(commands.contains(&Command::SaveState));
                let Request::Settings { values } = sent(&commands)[0] else { panic!("settings command") };
                assert_eq!(values.len(), 3);
            }
            assert_eq!((m.prefs.deadzone, m.prefs.anti_deadzone, m.prefs.stick_curve), (9, 1, Curve::Precise));
            assert_eq!(Preferences::read(&m.state.profile().ui), m.prefs);
            let values = &m.state.engines[&EngineId::new("gfn").unwrap()].settings;
            assert_eq!(values["deadzone"], Value::Int(9));
            assert_eq!(values["anti-deadzone"], Value::Int(1));
            assert_eq!(values["stick-curve"], Value::Text("precise".into()));
        }

        #[test]
        fn an_engines_background_error_stays_on_its_card_and_off_the_lab() {
            let mut m = lab();
            let engine = EngineId::new("gfn").unwrap();
            let error = |m: &mut Model| m.step(Input::Engine { engine: engine.clone(), message: FromEngine { re: Some(1), event: ipc::Event::Error {
                code: ipc::ErrorCode::Internal, message: "the installed app list exceeds the lifecycle limit".into() } } });
            error(&mut m);
            assert_eq!(m.view.page, Page::Lab);
            assert_eq!(m.view.notice, None);
            assert_eq!(m.integration(&engine).unwrap().error.as_deref(), Some("the installed app list exceeds the lifecycle limit"));
            // Where the engine is what the screen is about, the same error is still shown.
            m.view.notice = Some("No controller is connected.".into());
            m.view.selected_engine = Some(engine.clone());
            m.view.sheet = Some(Sheet::AddHost);
            error(&mut m);
            assert_eq!(m.view.notice.as_deref(), Some("the installed app list exceeds the lifecycle limit"));
            // A notice does not follow its page to another one.
            m.view.sheet = None;
            m.show_page(Page::Settings, Focus::Setting(0));
            assert_eq!(m.view.notice, None);
        }

        #[test]
        fn round_trip_is_additive_and_old_engines_have_no_measure_action() {
            let mut m = lab();
            session(&mut m, &[]);
            assert_eq!(m.pad_mode(), PadMode::Menu);
            assert_eq!(m.round_trip_engine(), None);
            assert!(!m.navigation.iter().any(|n| n.id == Focus::Lab(4)));
            let mut out = Vec::new();
            m.lab_action(4, &mut out);
            assert!(out.is_empty());
            session(&mut m, &[Feature::RoundTrip]);
            m.view.focus = Focus::Lab(4);
            let commands = key(&mut m, Key::Ok);
            assert!(matches!(sent(&commands).as_slice(), [Request::RoundTrip { session: SessionId(1) }]));
            let engine = EngineId::new("gfn").unwrap();
            let trip = ipc::RoundTrip { network_ms: Some(12.5), ..Default::default() };
            m.step(Input::Engine { engine: engine.clone(), message: FromEngine { re: None, event: ipc::Event::RoundTrip { session: SessionId(1), trip: trip.clone() } } });
            assert_eq!(m.lab.trip, Trip::Answer(engine.clone(), trip));
            key(&mut m, Key::Ok);
            m.step(Input::Engine { engine: engine.clone(), message: FromEngine { re: None, event: ipc::Event::Error { code: ipc::ErrorCode::Unsupported, message: "unknown request".into() } } });
            assert_eq!(m.lab.trip, Trip::Unsupported(engine));
            assert_eq!(m.view.notice, None);
            assert_eq!(m.view.focus, Focus::Setting(0));
            assert!(!m.navigation.iter().any(|n| n.id == Focus::Lab(4)));
        }

        #[test]
        fn gamesir_waits_for_readback_and_keeps_last_confirmed_values_on_error() {
            let mut m = lab();
            key(&mut m, Key::Back);
            m.view.focus = Focus::Setting(2);
            assert!(key(&mut m, Key::Ok).contains(&Command::GameSir(None)));
            assert_eq!(m.view.page, Page::GameSir);
            assert!(m.gamesir.pending);
            let ready = crate::game::helper::gamesir_fixture();
            m.step(Input::GameSir(Ok(ready.clone())));
            m.view.focus = Focus::Setting(1);
            let commands = m.step(Input::Key { key: Key::Right, down: true });
            assert!(commands.contains(&Command::GameSir(Some(ControllerChange { setting: "report-rate".into(), value: serde_json::json!(1000) }))));
            assert_eq!(m.gamesir.state, ready, "no optimistic hardware value");
            assert!(m.step(Input::Key { key: Key::Right, down: true }).is_empty(), "one request in flight");
            m.step(Input::GameSir(Err("Read-back did not match".into())));
            assert_eq!(m.gamesir.state, ready);
            assert!(!m.gamesir.pending);
            assert_eq!(m.view.focus, Focus::Setting(0), "the error is visible beside Refresh");
            m.view.focus = Focus::Setting(1);
            assert!(m.step(Input::Key { key: Key::Right, down: true }).is_empty(), "refresh is required after a failed write");
            m.view.focus = Focus::Setting(0);
            assert!(key(&mut m, Key::Ok).contains(&Command::GameSir(None)));
            m.step(Input::GameSir(Ok(GameSirState::Mode { device: "Wireless Controller".into() })));
            assert!(!m.navigation.iter().any(|n| n.id == Focus::Setting(1)));
            m.step(Input::GameSir(Ok(GameSirState::Off)));
            assert_eq!(m.gamesir.state, GameSirState::Off);
            key(&mut m, Key::Back);
            assert_eq!((m.view.page, m.view.focus), (Page::Settings, Focus::Setting(2)));
        }

        #[test]
        fn leadjoy_edits_wait_for_readback_and_keep_refresh_available() {
            let mut m = lab();
            key(&mut m, Key::Back);
            m.view.focus = Focus::Setting(3);
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: LeadjoyView::default(), change: None }));
            assert_eq!(m.view.page, Page::Leadjoy);
            let ready = crate::game::helper::leadjoy_fixture("general");
            m.step(Input::Leadjoy(Ok(ready.clone())));
            m.view.focus = Focus::Setting(4);
            assert!(m.step(Input::Key { key: Key::Left, down: true }).is_empty(), "the green channel is already zero");
            m.view.focus = Focus::Setting(2);
            assert!(m.step(Input::Key { key: Key::Right, down: true }).contains(&Command::Leadjoy { view: LeadjoyView::default(), change: Some(ControllerChange {
                setting: "logo-brightness".into(), value: serde_json::json!(81),
            }) }));
            assert_eq!(m.leadjoy.state, ready);
            assert!(m.step(Input::Key { key: Key::Right, down: true }).is_empty());
            m.step(Input::Leadjoy(Err("Read-back did not match".into())));
            assert_eq!(m.leadjoy.state, ready);
            assert_eq!(m.view.focus, Focus::Setting(0));
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: LeadjoyView::default(), change: None }));
            m.step(Input::Leadjoy(Ok(LeadjoyState::Unsupported { device: "Other".into(), firmware: "1".into() })));
            assert!(!m.navigation.iter().any(|n| n.id == Focus::Setting(1)));
            m.step(Input::Leadjoy(Ok(LeadjoyState::Off)));
            assert!(!m.leadjoy.pending);
            key(&mut m, Key::Back);
            assert_eq!((m.view.page, m.view.focus), (Page::Settings, Focus::Setting(3)));
        }

        #[test]
        fn leadjoy_selectors_reread_while_names_and_actions_go_to_the_helper() {
            use crate::game::helper::leadjoy_fixture;
            let mut m = lab();
            key(&mut m, Key::Back);
            m.view.focus = Focus::Setting(3);
            key(&mut m, Key::Ok);
            m.step(Input::Leadjoy(Ok(leadjoy_fixture("general"))));
            m.view.focus = Focus::Setting(1);
            let profiles = LeadjoyView { section: "profiles".into(), ..LeadjoyView::default() };
            assert!(m.step(Input::Key { key: Key::Right, down: true }).contains(&Command::Leadjoy { view: profiles.clone(), change: None }));
            m.step(Input::Leadjoy(Ok(leadjoy_fixture("profiles"))));
            m.view.focus = Focus::Setting(3);
            assert!(key(&mut m, Key::Ok).is_empty());
            assert_eq!((m.view.sheet, m.view.address.as_str()), (Some(Sheet::EditText), "Default"));
            m.view.address = "Racing".into();
            m.view.focus = Focus::SheetAction(0);
            let rename = ControllerChange { setting: "profile-name".into(), value: serde_json::json!("Racing") };
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: profiles.clone(), change: Some(rename) }));
            assert_eq!(m.view.sheet, None);
            m.step(Input::Leadjoy(Ok(leadjoy_fixture("profiles"))));
            m.view.focus = Focus::Setting(4);
            let activate = ControllerChange { setting: "profile-activate".into(), value: serde_json::Value::Null };
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: profiles, change: Some(activate) }));
        }

        #[test]
        fn leadjoy_macro_steps_reread_and_deleting_needs_confirmation() {
            use crate::game::helper::leadjoy_fixture;
            let mut m = lab();
            key(&mut m, Key::Back);
            m.view.focus = Focus::Setting(3);
            key(&mut m, Key::Ok);
            m.step(Input::Leadjoy(Ok(leadjoy_fixture("macros"))));
            let macros = LeadjoyView { section: "macros".into(), ..LeadjoyView::default() };
            m.view.focus = Focus::Setting(3);
            assert!(m.step(Input::Key { key: Key::Right, down: true }).contains(&Command::Leadjoy { view: LeadjoyView { step: 2, ..macros.clone() }, change: None }));
            let LeadjoyState::Ready(mut second) = leadjoy_fixture("macros") else { panic!("the fixture is ready") };
            second.view.step = 2;
            m.step(Input::Leadjoy(Ok(LeadjoyState::Ready(second))));
            m.view.focus = Focus::Setting(2);
            assert!(m.step(Input::Key { key: Key::Right, down: true }).contains(&Command::Leadjoy { view: LeadjoyView { button: 2, ..macros.clone() }, change: None }), "another macro starts at its first step");
            m.step(Input::Leadjoy(Ok(leadjoy_fixture("macros"))));
            m.view.focus = Focus::Setting(11);
            assert!(key(&mut m, Key::Ok).is_empty());
            assert_eq!((m.view.sheet, m.view.focus), (Some(Sheet::Delete), Focus::SheetAction(0)));
            assert!(key(&mut m, Key::Ok).is_empty(), "keeping is the default");
            assert_eq!((m.view.sheet, m.view.focus), (None, Focus::Setting(11)));
            key(&mut m, Key::Ok);
            m.view.focus = Focus::SheetAction(1);
            let delete = ControllerChange { setting: "delete-macro".into(), value: serde_json::Value::Null };
            assert!(key(&mut m, Key::Ok).contains(&Command::Leadjoy { view: macros, change: Some(delete) }));
            assert_eq!((m.view.sheet, m.leadjoy.pending), (None, true));
        }

        #[test]
        fn lab_navigation_reaches_every_reading_without_sending_commands() {
            let mut m = lab();
            reports(&mut m, 1_000_000_000, 1, vec![report(1_000_000_000, 0)]);
            m.prefs.text_percent = 200;
            m.view.focus = Focus::Lab(2);
            m.rebuild_navigation();
            for expected in [Focus::Lab(5), Focus::Lab(6), Focus::Lab(7), Focus::Setting(0)] {
                key(&mut m, Key::Down);
                assert_eq!(m.view.focus, expected);
                if matches!(expected, Focus::Lab(_)) { assert!(key(&mut m, Key::Ok).is_empty()); }
            }
        }
    }
}

#[cfg(test)]
mod settings_navigation_tests {
    use super::*;
    #[test]
    fn rails_reach_their_own_pane_and_long_panes_keep_their_column() {
        let mut m = Model::new(State::default(), Listing::default(), Vec::new());
        m.view.page = Page::Settings;
        m.view.section = Section::Controllers;
        m.view.focus = Focus::Rail(3);
        m.rebuild_navigation();
        m.step(Input::Key { key: Key::Right, down: true });
        assert_eq!(m.view.focus, Focus::Setting(0));
        assert_eq!(m.view.section, Section::Controllers);
        m.view.section = Section::Appearance;
        m.view.focus = Focus::Setting(5);
        m.rebuild_navigation();
        m.step(Input::Key { key: Key::Up, down: true });
        assert_eq!(m.view.focus, Focus::Setting(4));
    }
}
