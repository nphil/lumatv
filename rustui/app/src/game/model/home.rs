//! Home: the featured game, the shelves, and the first-run list of integrations while there are no games.
use super::nav::Slice;
use super::*;

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
/// The longest "Continue" or "Most played" shelf on Home. A service can report hundreds of played games; the Library page lists them all.
const HOME_ROW_MAX: usize = 40;
pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, _layer: Option<Layer>) -> Vec<Vec<Focus>> {
        m.home_grid()
    }
    fn anchor(m: &Model) -> Option<Focus> {
        m.home_grid().into_iter().flatten().next()
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        m.home_activate(f, out);
    }
    fn adjust(_m: &mut Model, _f: Focus, _forward: bool, _out: &mut Vec<Command>) -> bool {
        false
    }
    fn subject(m: &Model) -> Option<GameId> {
        match m.view.focus {
            Focus::Hero(_) => m.view.featured,
            Focus::Home(r, i) => match m.home_rows.get(r).and_then(|row| row.items.get(i)) {
                Some(HomeItem::Game(id)) => Some(*id),
                _ => None,
            },
            _ => None,
        }
    }
}
impl Model {
    fn home_grid(&self) -> Vec<Vec<Focus>> {
        if self.games.is_empty() {
            return vec![(0..self.integrations.len()).map(Focus::FirstRun).collect()];
        }
        let mut rows: Vec<Vec<Focus>> = Vec::new();
        if self.prefs.text_percent > 100 { rows.extend([vec![Focus::Hero(0)], vec![Focus::Hero(1)], vec![Focus::Hero(2)]]); } else { rows.push(vec![Focus::Hero(0), Focus::Hero(1), Focus::Hero(2)]); }
        for (r, row) in self.home_rows.iter().enumerate() {
            rows.push((0..row.items.len()).map(|i| Focus::Home(r, i)).collect());
        }
        rows
    }
    fn home_activate(&mut self, f: Focus, out: &mut Vec<Command>) {
        match f {
            Focus::Hero(0) => {
                if let Some(id) = self.view.featured {
                    self.view.selected = Some(id);
                    self.play(id, out);
                }
            }
            Focus::Hero(2) => self.refresh_libraries(None, out),
            Focus::Hero(_) => {
                if let Some(id) = self.view.featured {
                    self.open_game(id, None);
                }
            }
            Focus::Home(r, i) => match self.home_rows.get(r).and_then(|r| r.items.get(i)).cloned() {
                Some(HomeItem::Game(id)) => self.open_game(id, None),
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
            Focus::FirstRun(i) => self.open_integration(i),
            _ => {}
        }
    }
    pub(super) fn rebuild_rows(&mut self) {
        self.rebuild_genres();
        let profile = self.state.profile();
        let valid = |id: &GameId| self.games.iter().any(|g| g.id == *id) && !profile.user.hidden.contains(id);
        let mut rows = Vec::new();
        // The shelves read this TV's history and every service's reports together (see `activity`).
        let shelf = |ids: Vec<GameId>, most: usize| ids.into_iter().filter(|id| !profile.user.hidden.contains(id)).take(most).map(HomeItem::Game).collect::<Vec<_>>();
        let recent = shelf(activity::recently_played(&self.games, &profile.history), HOME_ROW_MAX);
        if !recent.is_empty() {
            rows.push(HomeRow { title: "Continue".into(), items: recent });
        }
        let most = shelf(activity::most_played(&self.games, &profile.history), HOME_ROW_MAX);
        if !most.is_empty() {
            rows.push(HomeRow { title: "Most played".into(), items: most });
        }
        let favourites = shelf(activity::favourites(&self.games, &profile.user.favourites), usize::MAX);
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
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::tests::*;
    #[test]
    fn no_history_means_no_empty_home_rows_and_idle_tick_does_not_relayout() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        assert!(m.home_rows.is_empty());
        let r = m.revision();
        assert!(m.step(Input::Tick { now_ms: 1000, unix_s: 100 }).is_empty());
        assert_eq!(m.revision(), r);
    }
    #[test]
    fn steam_play_merges_with_this_tvs_history_for_the_shelf_the_sort_and_the_genre_filter() {
        use lumagame_core::activity::{Activity, Place};
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("pc", "PC", EngineKind::Host), manifest("steam", "Steam", EngineKind::Account)]);
        describe(&mut m, "pc", vec![], &[]);
        describe(&mut m, "steam", vec![lumagame_core::engine::AuthMethod::Qr], &[]);
        let account = "76561198000000000";
        let asked = engine_says(&mut m, "steam", ipc::Event::Targets { targets: vec![linked(account)] });
        assert!(matches!(sent(&asked)[..], [Request::Library { cursor: None, .. }]), "a linked account is asked for its library at once");
        let record = |appid: &str, activity| entry(appid, appid, ipc::Role::Record, &[&format!("steam:{appid}")], activity);
        let records = vec![
            record("10", Activity { last_played: Some(5_000), playtime_s: Some(7_200), genres: vec!["Action".into(), "ROLE_PLAYING".into()], tags: vec!["Open World".into()], ..Activity::default() }),
            record("20", Activity { last_played: Some(9_000), genres: vec!["Role-playing (RPG)".into()], ..Activity::default() }),
            record("99", Activity { last_played: Some(99_000), ..Activity::default() }),
        ];
        engine_says(&mut m, "steam", ipc::Event::Library { target: TargetId::new(account).unwrap(), entries: records, next: None });
        engine_says(&mut m, "pc", ipc::Event::Targets { targets: vec![linked("main")] });
        let games = vec![
            entry("a", "Alpha", ipc::Role::Game, &["steam:10"], Activity::default()),
            entry("b", "Beta", ipc::Role::Game, &["steam:20"], Activity::default()),
            entry("c", "Gamma", ipc::Role::Game, &[], Activity::default()),
        ];
        engine_says(&mut m, "pc", ipc::Event::Library { target: TargetId::new("main").unwrap(), entries: games, next: None });
        let (alpha, beta, gamma) = (id_of(&m, "Alpha"), id_of(&m, "Beta"), id_of(&m, "Gamma"));
        assert_eq!(m.games().len(), 3, "a Steam game the TV does not list is not a card");
        m.state.profile_mut().history.record(Played { game: gamma, source: "pc|main|c".parse().unwrap(), start: 7_000, seconds: 60 });
        m.unix_s = 9_000 + 3 * 86_400;
        m.rebuild_rows();
        m.rebuild_filter();

        let shelf = |m: &Model, title: &str| m.home_rows.iter().find(|r| r.title == title).map(|r| r.items.clone());
        assert_eq!(shelf(&m, "Continue"), Some(vec![HomeItem::Game(beta), HomeItem::Game(gamma), HomeItem::Game(alpha)]), "Steam's 9000 beats this TV's 7000 beats Steam's 5000");
        assert_eq!(shelf(&m, "Most played"), Some(vec![HomeItem::Game(alpha), HomeItem::Game(gamma)]), "7200 s on Steam outranks 60 s here; unplayed Beta is absent");
        assert_eq!(m.last_played(beta).map(|l| l.place), Some(Place::Service(EngineId::new("steam").unwrap())));
        assert_eq!(m.since_played(beta).as_deref(), Some("3 days ago"));
        assert_eq!(m.play_time_of(alpha), Some((7_200, Place::Service(EngineId::new("steam").unwrap()))));
        assert_eq!(m.game_tags(alpha), ["Open World"]);
        assert_eq!(m.genres(), ["Action", "RPG"], "two spellings of one genre are one label");
        m.view.sort = Sort::Recent;
        m.rebuild_filter();
        assert_eq!(m.filtered, [beta, gamma, alpha]);
        m.view.filter.genre = Some("RPG".into());
        m.rebuild_filter();
        assert_eq!(m.filtered, [beta, alpha], "both spellings fall under the one genre");
        m.view.filter.genre = Some("Action".into());
        m.rebuild_filter();
        assert_eq!(m.filtered, [alpha]);
    }
    #[test]
    fn a_service_favourite_joins_the_owners_favourites_without_becoming_one() {
        use lumagame_core::activity::Activity;
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud)]);
        describe(&mut m, "gfn", vec![], &[]);
        engine_says(&mut m, "gfn", ipc::Event::Targets { targets: vec![linked("main")] });
        let entries = vec![
            entry("1", "Alpha", ipc::Role::Game, &[], Activity { favourite: true, ..Activity::default() }),
            entry("2", "Beta", ipc::Role::Game, &[], Activity::default()),
            entry("3", "Gamma", ipc::Role::Game, &[], Activity::default()),
        ];
        engine_says(&mut m, "gfn", ipc::Event::Library { target: TargetId::new("main").unwrap(), entries, next: None });
        let (alpha, beta, gamma) = (id_of(&m, "Alpha"), id_of(&m, "Beta"), id_of(&m, "Gamma"));
        m.state.profile_mut().user.favourites.insert(beta);
        m.rebuild_rows();
        m.view.filter.favourites = true;
        m.rebuild_filter();
        let favourites = m.home_rows.iter().find(|r| r.title == "Favourites").map(|r| r.items.clone());
        assert_eq!(favourites, Some(vec![HomeItem::Game(beta), HomeItem::Game(alpha)]));
        assert_eq!(m.filtered, [alpha, beta], "the filter takes both, in title order");
        assert_eq!((m.is_favourite(alpha), m.is_favourite(gamma)), (true, false));
        assert_eq!(m.favourite_on(alpha), ["GeForce NOW"]);
        assert!(!m.state().profile().user.favourites.contains(&alpha), "the heart edits only the owner's own set");
    }
}
