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
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

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
    pub stats: bool,
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
            stats: false,
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
        p.stats = v["stats"].as_bool().unwrap_or(false);
        p
    }
    fn json(&self) -> serde_json::Value {
        serde_json::json!({"family":self.family,"light":self.light,"hero":self.hero.index(),"covers":self.covers.index(),"text_percent":self.text_percent,"reduce_motion":self.reduce_motion,"width":self.width,"height":self.height,"fps":self.fps,"hdr":self.hdr,"codec":self.codec.map_or("auto",Codec::as_str),"bitrate_mbps":self.bitrate_mbps,"stats":self.stats})
    }
    pub fn text_scale(&self) -> f32 {
        f32::from(self.text_percent) / 100.0
    }
}
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
            integrations: manifests
                .into_iter()
                .map(|manifest| Integration { manifest, descriptor: None, schema: SettingsSchema::default(), targets: Vec::new(), error: None })
                .collect(),
            prefs,
            view: View::default(),
            details: BTreeMap::new(),
            art: BTreeMap::new(),
            home_rows: Vec::new(),
            filtered: Vec::new(),
            navigation: Vec::new(),
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
        self.foreground && self.view.page != Page::Running
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
            Input::Engine { engine, message } => self.engine_event(engine, message.event, &mut out),
            Input::EngineGone(id) => {
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
                if !on {
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
                if self.view.auth.as_ref().is_some_and(|a| a.expires_ms.is_some_and(|end| now_ms >= end) && !matches!(a.state, Some(AuthState::Failed { .. })))
                {
                    if let Some(auth) = &mut self.view.auth {
                        auth.state = Some(AuthState::Failed { reason: ipc::AuthFail::Expired, message: "The code expired. Start sign-in again.".into() });
                    }
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
        }
        self.view.page = page;
        self.view.sheet = None;
        self.view.focus = focus;
        self.pressed = None;
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
        if self.view.page == Page::Running {
            if key == Key::Guide && !down {
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
                if self.view.page == Page::Integration {
                    self.adjust(true, out);
                } else if self.view.section == Section::Appearance && i == 0 {
                    self.open_sheet(Sheet::Theme, Focus::Theme(0));
                } else if self.view.section == Section::Controllers {
                    out.push(Command::OpenControllerSettings);
                } else if self.view.section == Section::About {
                    out.push(Command::PlayMedia { url: ["https://store.steampowered.com", "https://steamgriddb.com", "https://igdb.com"][i.min(2)].into() });
                } else {
                    self.adjust(true, out);
                }
            }
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
                Some(Sheet::Auth) => self.auth_action(out),
                Some(Sheet::EditText) => self.save_text(out),
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
                _ => self.open_sheet(Sheet::EndSession, Focus::SheetAction(0)),
            },
            Focus::None => {}
        }
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
                6 => self.prefs.stats = !self.prefs.stats,
                _ => return false,
            },
            _ => return false,
        }
        self.save_prefs(out);
        if self.view.section == Section::Streaming {
            let desired = self.stream_values();
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
        true
    }
    fn save_prefs(&mut self, out: &mut Vec<Command>) {
        self.state.profile_mut().ui = self.prefs.json();
        out.push(Command::SaveState);
    }
    fn stream_values(&self) -> Values {
        Values::from([
            ("width".into(), Value::Int(i64::from(self.prefs.width))),
            ("height".into(), Value::Int(i64::from(self.prefs.height))),
            ("fps".into(), Value::Int(i64::from(self.prefs.fps))),
            ("hdr".into(), Value::Bool(self.prefs.hdr)),
            ("bitrate-mbps".into(), Value::Int(i64::from(self.prefs.bitrate_mbps))),
            ("codec".into(), Value::Text(self.prefs.codec.map_or("auto", Codec::as_str).into())),
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
            Self::send(out, &s.source.engine, Request::Stats { session: s.id, on: true });
        }
    }
    fn resume(&mut self, out: &mut Vec<Command>) {
        if let Some(s) = &self.session {
            self.view.page = Page::Running;
            self.view.sheet = None;
            self.view.focus = Focus::None;
            Self::send(out, &s.source.engine, Request::Route { session: s.id, input: ipc::InputRoute::Game });
            Self::send(out, &s.source.engine, Request::Stats { session: s.id, on: self.prefs.stats });
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
        self.view.auth = Some(LinkFlow { engine: engine.clone(), target: target.clone(), state: None, expires_ms: None, pin: String::new() });
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
    fn auth_action(&mut self, out: &mut Vec<Command>) {
        let Some(a) = &self.view.auth else {
            return;
        };
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
                self.auth_action(out);
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
                    let desired = self.stream_values();
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
                for t in targets.iter().filter(|t| t.link == ipc::Link::Linked) {
                    self.pages.insert((id.clone(), t.id.clone()), Vec::new());
                    Self::send(out, &id, Request::Library { target: t.id.clone(), cursor: None });
                }
                if let Some(e) = self.integrations.iter_mut().find(|e| e.manifest.id == id) {
                    e.targets = targets;
                }
                self.rebuild_catalog();
            }
            ipc::Event::Library { target, entries, next } => {
                let page = self.pages.entry((id.clone(), target.clone())).or_default();
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
                if self.view.auth.as_ref().is_none_or(|a| a.engine != id) {
                    self.view.auth = Some(LinkFlow { engine: id.clone(), target: None, state: None, expires_ms: None, pin: String::new() });
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
                    }
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
            ipc::Event::Error { message, .. } => {
                self.view.notice = Some(message.clone());
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
                for (row_index, row) in self.home_rows.iter().enumerate().skip(self.view.home_row).take(2) {
                    let first = if let Focus::Home(r, i) = self.view.focus {
                        if r == row_index { i.saturating_sub(self.prefs.covers.columns() - 2) } else { 0 }
                    } else {
                        0
                    };
                    for item in row.items.iter().skip(first).take(self.prefs.covers.columns() + 1) {
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
                self.filtered.iter().skip(self.view.first_grid_row * self.prefs.covers.columns()).take(self.prefs.covers.columns() * 2).copied().collect()
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
                Sheet::EndSession => rows.push(vec![Focus::SheetAction(0), Focus::SheetAction(1)]),
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
                Sheet::Auth => rows.push(vec![Focus::SheetAction(0)]),
            }
        } else {
            match self.view.page {
                Page::Home | Page::Library | Page::Search | Page::Settings | Page::Integration => rows.push((0..4).map(Focus::Nav).collect()),
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
                        Section::Controllers => 1,
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
                Page::Overlay => rows.push((0..3).map(Focus::Overlay).collect()),
                Page::Media => rows.push(vec![Focus::HubTop(0)]),
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
mod tests {
    use super::*;
    fn key(m: &mut Model, key: Key) -> Vec<Command> {
        m.step(Input::Key { key, down: true });
        m.step(Input::Key { key, down: false })
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
        m.view.focus = Focus::Overlay(2);
        m.rebuild_navigation();
        key(&mut m, Key::Ok);
        assert_eq!(m.view.sheet, Some(Sheet::EndSession));
        assert_eq!(m.view.focus, Focus::SheetAction(0));
        key(&mut m, Key::Back);
        assert_eq!(m.view.focus, Focus::Overlay(2));
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
