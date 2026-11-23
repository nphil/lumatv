//! The game's page: its actions, sections and ways to play, the chooser, and what the page knows beyond its artwork (the sections
//! it offers, only those with something to show, the owner's own achievements from an engine that can read them, and the quiet
//! look-ahead that fetches a game's page data while its tile rests in focus).
pub mod media;

use super::facts::{Ask, Reader, Section, steam_app};
use super::nav::Slice;
use super::settings::Place;
use super::*;
use lumagame_core::launch::{self, Candidate, Resolution};
use lumagame_meta::hub::{Deck, Hub};
use lumagame_core::ids::ProviderGameId;

/// How long a tile must stay focused before its page data is fetched ahead of the owner opening it.
pub(super) const REST_MS: u64 = 700;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubTab {
    Overview,
    Ways,
    Achievements,
    News,
    Media,
    Details,
}
impl HubTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Ways => "Ways to play",
            Self::Achievements => "Achievements",
            Self::News => "News",
            Self::Media => "Media",
            Self::Details => "Details",
        }
    }
}

impl GameDetails {
    /// The page data a game's public sources gave. Nothing is invented: a part the sources did not give stays empty, and the page hides it.
    pub fn from_hub(hub: Hub) -> Self {
        let year = hub.release.as_deref().and_then(|r| r.rsplit(|c: char| !c.is_ascii_digit()).find(|s| s.len() == 4)).and_then(|y| y.parse().ok());
        let mut capabilities = Vec::new();
        match hub.deck {
            Some(Deck::Verified) => capabilities.push("Steam Deck Verified".to_owned()),
            Some(Deck::Playable) => capabilities.push("Steam Deck Playable".to_owned()),
            _ => {}
        }
        capabilities.extend(hub.controller.clone());
        capabilities.extend(["Ultrawide", "HDR", "4K", "Adaptive triggers"].into_iter().filter(|label| hub.facts.iter().any(|f| f.label == *label && f.value == "Yes")).map(str::to_owned));
        let players: Vec<&str> = [("Single-player", "Single-player"), ("Multi-player", "Multiplayer"), ("Co-op", "Co-op")]
            .into_iter()
            .filter(|(category, _)| hub.categories.iter().any(|c| c.starts_with(category)))
            .map(|(_, label)| label)
            .collect();
        let trailers = hub.trailers.iter().map(|t| Media { title: t.title.clone(), image: t.thumb.clone(), video: Some(t.url.clone()) });
        let shots = hub.screenshots.iter().enumerate().map(|(i, url)| Media { title: format!("Screenshot {}", i + 1), image: url.clone(), video: None });
        Self {
            description: hub.description.clone(),
            year,
            genres: hub.genres.clone(),
            players: (!players.is_empty()).then(|| players.join(", ")),
            developer: (!hub.developers.is_empty()).then(|| hub.developers.join(", ")),
            capabilities,
            cloud_saves: hub.categories.iter().any(|c| c == "Steam Cloud").then_some(true),
            media: trailers.chain(shots).collect(),
            hub,
        }
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
pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        m.game_grid(layer)
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        m.game_activate(f, out);
    }
    fn adjust(_m: &mut Model, _f: Focus, _forward: bool, _out: &mut Vec<Command>) -> bool {
        false
    }
    fn anchor(_m: &Model) -> Option<Focus> {
        Some(Focus::HubAction(0))
    }
    fn subject(m: &Model) -> Option<GameId> {
        m.view.selected
    }
}

impl Model {
    /// The sections the open game's page offers, in order. A section with nothing to show is not offered.
    pub fn hub_tabs(&self) -> Vec<HubTab> {
        let mut tabs = vec![HubTab::Overview, HubTab::Ways];
        let Some(id) = self.view.selected else { return tabs };
        let details = self.details.get(&id);
        if details.is_some_and(|d| d.hub.achievements.total > 0) || self.owned.get(&id).is_some_and(|u| !u.is_empty()) {
            tabs.push(HubTab::Achievements);
        }
        if details.is_some_and(|d| !d.hub.news.is_empty()) {
            tabs.push(HubTab::News);
        }
        if details.is_some_and(|d| !d.media.is_empty()) {
            tabs.push(HubTab::Media);
        }
        if details.is_some_and(|d| {
            let h = &d.hub;
            !(h.developers.is_empty() && h.publishers.is_empty() && h.release.is_none() && h.categories.is_empty() && h.facts.is_empty() && h.deck.is_none() && h.credits.is_empty())
        }) {
            tabs.push(HubTab::Details);
        }
        tabs
    }

    pub fn hub_tab(&self) -> HubTab {
        self.hub_tabs().get(self.view.hub_tab).copied().unwrap_or(HubTab::Overview)
    }

    /// What the owner played of the game in the last two weeks, in seconds, where a service says so.
    pub fn recent_play(&self, id: GameId) -> Option<u64> {
        lumagame_core::activity::recent_play(self.game(id)?)
    }

    /// The owner's own unlocked achievements in the game, when an engine has said.
    pub fn unlocked(&self, id: GameId) -> Option<&[ipc::Unlocked]> {
        self.owned.get(&id).map(Vec::as_slice)
    }

    /// Asks the engine that can read the owner's achievements in the game, once until the app comes back from the background.
    pub(super) fn request_owned(&mut self, id: GameId, out: &mut Vec<Command>) {
        if self.requested_owned.contains(&id) {
            return;
        }
        let Some(game) = self.game(id) else { return };
        let Reader::Linked { engine, target, ask } = self.reader(game) else { return };
        let Ok(game) = (match ask {
            Ask::Source(own) => Ok(own.clone()),
            Ask::App(app) => ProviderGameId::new(app.to_string()),
        }) else {
            return;
        };
        let (engine, target) = (engine.clone(), target.clone());
        self.requested_owned.insert(id);
        Self::send(out, &engine, Request::Achievements { target, game });
    }

    /// An answer to `request_owned`. An engine that lists the game names it by its own id; Steam lists a library as records, which are
    /// not games, so it is asked by the game's Steam app id and answers with that.
    pub(super) fn owned_achievements(&mut self, engine: &EngineId, target: &TargetId, game: &ProviderGameId, unlocked: Vec<ipc::Unlocked>) {
        let own = self.games.iter().find(|g| g.sources.iter().any(|s| s.engine == *engine && s.target == *target && s.entry.id == *game));
        let found = own.or_else(|| {
            let app = game.as_str().parse::<u32>().ok()?;
            self.games.iter().find(|g| steam_app(g) == Some(app))
        });
        if let Some(id) = found.map(|g| g.id) {
            self.owned.insert(id, unlocked);
        }
    }

    /// A tile that stays focused for a moment gets its page data fetched, so the page is ready when it opens.
    pub(super) fn look_ahead(&mut self, out: &mut Vec<Command>) {
        let Some(id) = self.view.featured.filter(|_| matches!(self.view.page, super::Page::Home | super::Page::Library)) else {
            self.resting = None;
            return;
        };
        match self.resting {
            Some((rested, since)) if rested == id => {
                if self.now_ms.saturating_sub(since) >= REST_MS && !self.details.contains_key(&id) && self.requested_details.insert(id) {
                    out.push(Command::FetchDetails { game: id });
                }
            }
            _ => self.resting = Some((id, self.now_ms)),
        }
    }
    fn game_grid(&self, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        let mut rows: Vec<Vec<Focus>> = Vec::new();
        match layer {
            Some(Layer::Chooser) => {
                if let Some(g) = self.selected_game() {
                    rows.extend((0..g.sources.len()).map(|i| vec![Focus::SheetRow(i)]));
                }
                rows.push(vec![Focus::Remember]);
                rows.push(vec![Focus::SheetAction(0)]);
            }
            Some(_) => {}
            None => {
                rows.push((0..3).map(Focus::HubTop).collect());
                if self.view.hub_tab == 0 {
                    if self.prefs.text_percent > 100 { rows.extend([vec![Focus::HubAction(0)], vec![Focus::HubAction(1)]]); } else { rows.push(vec![Focus::HubAction(0), Focus::HubAction(1)]); }
                }
                rows.push((0..self.hub_tabs().len()).map(Focus::Tab).collect());
                let items = self.view.selected.and_then(|id| self.details.get(&id)).map_or(0, |d| match self.hub_tab() {
                    HubTab::Achievements => d.hub.achievements.shown.len(),
                    HubTab::News => d.hub.news.len(),
                    _ => 0,
                });
                rows.extend((0..items).map(|i| vec![Focus::Item(i)]));
                if self.hub_tab() == HubTab::Ways {
                    rows.extend((0..=self.selected_game().map_or(0, |g| g.sources.len())).map(|i| vec![Focus::Way(i)]));
                }
                if self.hub_tab() == HubTab::Media {
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
        }
        rows
    }
    fn game_activate(&mut self, f: Focus, out: &mut Vec<Command>) {
        match f {
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
            Focus::HubAction(0) => {
                if let Some(id) = self.view.selected {
                    self.play(id, out);
                }
            }
            Focus::HubAction(_) => self.open_chooser(),
            Focus::Tab(tab) => {
                self.view.hub_tab = tab;
            }
            Focus::Item(i) => {
                let url = self.view.selected.and_then(|id| self.details.get(&id)).filter(|_| self.hub_tab() == HubTab::News).and_then(|d| d.hub.news.get(i)).map(|n| n.url.clone());
                if let Some(url) = url {
                    out.push(Command::PlayMedia { url });
                }
            }
            Focus::Way(i) => {
                if let Some(game) = self.selected_game() {
                    if let Some(s) = game.sources.get(i) {
                        self.view.chooser_selected = Some(s.key());
                        self.view.hub_tab = 0;
                        self.view.focus = Focus::HubAction(0);
                    } else {
                        self.open_settings(Place::Accounts);
                    }
                }
            }
            Focus::Media(i) => self.open_media(i, out),
            Focus::SheetRow(i) if self.view.top() == Some(Layer::Chooser) => {
                if let Some(s) = self.selected_game().and_then(|g| g.sources.get(i)) {
                    self.view.chooser_selected = Some(s.key());
                    self.view.focus = Focus::SheetAction(0);
                }
            }
            Focus::Remember => self.view.remember = !self.view.remember,
            Focus::SheetAction(_) if self.view.top() == Some(Layer::Chooser) => {
                if let (Some(game), Some(source)) = (self.view.selected, self.view.chooser_selected.clone()) {
                    if self.readiness(&source).can_attempt() {
                        self.start(game, source, self.view.remember, out);
                    } else {
                        self.view.notice = Some("This target is unavailable. Choose another way to play.".into());
                    }
                }
            }
            _ => {}
        }
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
    /// Opens the game's page. `_section` is where the page opens; until the page has sections it always opens at the top.
    pub fn open_game(&mut self, id: GameId, _section: Option<Section>) {
        self.view.selected = Some(id);
        self.view.chooser_selected = None;
        self.view.hub_tab = 0;
        self.view.notice = None;
        self.show_page(Page::Game, Focus::HubAction(0));
    }
    fn open_chooser(&mut self) {
        if let Some(id) = self.view.selected {
            let desired = self.play_target(id);
            self.view.chooser_selected = desired.clone();
            let i = self.game(id).and_then(|g| g.sources.iter().position(|s| Some(s.key()) == desired)).unwrap_or(0);
            self.view.remember = self.state.profile().user.pinned.contains_key(&id);
            self.push_layer(Layer::Chooser, Focus::SheetRow(i));
        }
    }
    /// Plays `game` the way it was chosen or remembered, else asks which way.
    pub fn play(&mut self, game: GameId, out: &mut Vec<Command>) {
        if self.view.selected != Some(game) {
            self.view.selected = Some(game);
            self.view.chooser_selected = None;
        }
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
            Resolution::Nothing => self.open_settings(Place::Accounts),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::tests::*;
    #[test]
    fn a_game_page_offers_only_the_sections_that_have_data() {
        use lumagame_meta::hub::{Achievements, Hub, News};
        let (mut m, _) = refreshing_steam();
        let game = id_of(&m, "Beta");
        m.view.selected = Some(game);
        m.view.page = Page::Game;
        assert_eq!(m.hub_tabs(), [HubTab::Overview, HubTab::Ways], "nothing fetched: no empty sections");
        m.step(Input::Details { game, details: Box::new(GameDetails::from_hub(Hub { credits: vec!["x".into()], ..Hub::default() })) });
        assert_eq!(m.hub_tabs(), [HubTab::Overview, HubTab::Ways, HubTab::Details]);
        let hub = Hub {
            achievements: Achievements { total: 5, ..Achievements::default() },
            news: vec![News { title: "t".into(), author: String::new(), date: 1, url: "https://x".into(), summary: String::new() }],
            screenshots: vec!["https://x/a.jpg".into()],
            credits: vec!["x".into()],
            developers: vec!["d".into()],
            ..Hub::default()
        };
        m.step(Input::Details { game, details: Box::new(GameDetails::from_hub(hub)) });
        assert_eq!(m.hub_tabs(), [HubTab::Overview, HubTab::Ways, HubTab::Achievements, HubTab::News, HubTab::Media, HubTab::Details]);
        m.view.hub_tab = 3;
        m.view.focus = Focus::Item(0);
        m.rebuild_navigation();
        assert!(key(&mut m, Key::Ok).iter().any(|c| matches!(c, Command::PlayMedia { url } if url == "https://x")), "OK on a news row opens it");
    }
    #[test]
    fn the_owners_achievements_are_asked_for_once_per_visit_and_kept() {
        use lumagame_core::ids::ProviderGameId;
        let (mut m, account) = refreshing_steam();
        describe(&mut m, "steam", vec![lumagame_core::engine::AuthMethod::Qr], &[Feature::Achievements]);
        let game = id_of(&m, "Beta");
        m.view.selected = Some(game);
        m.view.page = Page::Game;
        let asked = m.step(Input::Notice("x".into()));
        assert!(matches!(sent(&asked)[..], [Request::Achievements { .. }]));
        assert!(sent(&m.step(Input::Notice("y".into()))).is_empty(), "not again on every redraw");
        engine_says(&mut m, "steam", ipc::Event::Achievements { target: account, game: ProviderGameId::new("0").unwrap(), unlocked: vec![ipc::Unlocked { name: "ACH_A".into(), at: None }] });
        assert_eq!(m.unlocked(game).map(<[_]>::len), Some(1));
        assert_eq!(m.hub_tabs(), [HubTab::Overview, HubTab::Ways, HubTab::Achievements]);
        let back = m.step(Input::Foreground(true));
        assert_eq!(sent(&back).iter().filter(|r| matches!(r, Request::Achievements { .. })).count(), 1, "asked again after the app comes back");
    }
    #[test]
    fn the_owners_achievements_in_a_game_steam_lists_only_as_a_record_are_asked_by_its_app_id() {
        use lumagame_core::activity::Activity;
        use lumagame_core::ids::ProviderGameId;
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud), manifest("steam", "Steam", EngineKind::Account)]);
        describe(&mut m, "gfn", vec![lumagame_core::engine::AuthMethod::DeviceCode], &[]);
        describe(&mut m, "steam", vec![lumagame_core::engine::AuthMethod::Qr], &[Feature::Achievements]);
        let account = TargetId::new("76561198000000000").unwrap();
        engine_says(&mut m, "steam", ipc::Event::Targets { targets: vec![linked("76561198000000000")] });
        engine_says(&mut m, "gfn", ipc::Event::Targets { targets: vec![linked("main")] });
        let hades = entry("1", "Hades II", ipc::Role::Game, &["steam:1145350"], Activity::default());
        engine_says(&mut m, "gfn", ipc::Event::Library { target: TargetId::new("main").unwrap(), entries: vec![hades], next: None });
        let game = id_of(&m, "Hades II");
        m.view.selected = Some(game);
        m.view.page = Page::Game;
        let asked = m.step(Input::Notice("x".into()));
        assert!(matches!(sent(&asked)[..], [Request::Achievements { target, game }] if *target == account && game.as_str() == "1145350"));
        let unlocked = vec![ipc::Unlocked { name: "ACH_A".into(), at: None }];
        engine_says(&mut m, "steam", ipc::Event::Achievements { target: account, game: ProviderGameId::new("1145350").unwrap(), unlocked });
        assert_eq!(m.unlocked(game).map(<[_]>::len), Some(1));
    }
    #[test]
    fn a_tile_that_rests_in_focus_gets_its_page_data_fetched_ahead() {
        let (mut m, _) = refreshing_steam();
        let game = id_of(&m, "Beta");
        m.view.featured = Some(game);
        let tick = |m: &mut Model, ms| m.step(Input::Tick { now_ms: ms, unix_s: 0 });
        let fetches = |c: &[Command]| c.iter().filter(|c| matches!(c, Command::FetchDetails { .. })).count();
        assert_eq!(fetches(&tick(&mut m, 10_000)), 0);
        assert_eq!(fetches(&tick(&mut m, 10_600)), 0, "not yet");
        assert_eq!(fetches(&tick(&mut m, 10_700)), 1);
        assert_eq!(fetches(&tick(&mut m, 11_700)), 0, "once");
    }
    #[test]
    fn play_time_reads_in_minutes_under_an_hour() {
        assert_eq!(play_time(30), "Under a minute");
        assert_eq!(play_time(600), "10 min");
        assert_eq!(play_time(3600), "1 h");
        assert_eq!(play_time(5400), "1 h 30 min");
    }
}
