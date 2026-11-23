//! Home: one row of the owner's games under the top bar, and behind it the focused game's art, logo, one line and Play. One press down
//! opens that game's page, which holds its activity, community, media and about; Back there is Home.
use super::facts::Fact;
use super::nav::Slice;
use super::*;

/// The longest the row gets. A service can report hundreds of played games; the Library page lists them all.
const TILES: usize = 40;
/// How many tiles the row shows at once, the focused one first.
pub const TILES_SHOWN: usize = 6;

/// Where focus rests on Home.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Spot {
    /// A game of the row, by its place in `Model::home_games`.
    Tile(usize),
    /// The empty Home's one button.
    Add,
}

pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, _layer: Option<Layer>) -> Vec<Vec<Focus>> {
        if m.home_games.is_empty() {
            return vec![vec![Focus::Home(Spot::Add)]];
        }
        vec![(0..m.home_games.len()).map(|i| Focus::Home(Spot::Tile(i))).collect()]
    }
    fn default_focus(m: &Model, region: Region) -> Option<Focus> {
        match region {
            Region::Page => Some(m.home_focus()),
            Region::Top => Some(Focus::Nav(m.tab())),
            Region::Layer => None,
        }
    }
    fn anchor(m: &Model) -> Option<Focus> {
        Some(m.home_focus())
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        match f {
            Focus::Home(Spot::Tile(i)) => {
                if let Some(id) = m.home_games.get(i).copied() {
                    m.play(id, out);
                }
            }
            Focus::Home(Spot::Add) => m.open_settings(settings::Place::Accounts),
            _ => {}
        }
    }
    fn adjust(_m: &mut Model, _f: Focus, _forward: bool, _out: &mut Vec<Command>) -> bool {
        false
    }
    fn subject(m: &Model) -> Option<GameId> {
        match m.view.focus {
            Focus::Home(Spot::Tile(_)) => m.view.featured,
            _ => None,
        }
    }
    fn hints(m: &Model) -> Hints {
        match m.view.focus {
            Focus::Home(Spot::Tile(_)) => Hints(vec![(Key::Down, "More"), (Key::Menu, "Menu")]),
            _ => Hints(vec![(Key::Menu, "Menu")]),
        }
    }
}
impl Model {
    /// The featured game's place in the row.
    pub fn home_tile(&self) -> Option<usize> {
        let id = self.view.featured?;
        self.home_games.iter().position(|g| *g == id)
    }
    /// Where focus rests on arriving at Home: the featured game's tile (the last played, until another is focused), else the empty
    /// Home's button.
    pub fn home_focus(&self) -> Focus {
        if self.home_games.is_empty() { Focus::Home(Spot::Add) } else { Focus::Home(Spot::Tile(self.home_tile().unwrap_or(0))) }
    }
    /// The stage's one line: "Played 2 h ago · 31 h · 18 of 50 achievements", any part that is not known left out.
    pub fn home_line(&self, id: GameId) -> String {
        let facts = self.facts(id);
        let mut parts = Vec::new();
        if let Some(ago) = self.since_played(id) {
            parts.push(format!("Played {}", ago.to_lowercase()));
        }
        if let Fact::Present(time) = &facts.activity.play_time {
            parts.push(game::play_time(time.seconds));
        }
        if let Fact::Present(a) = &facts.activity.achievements {
            parts.push(match a.total {
                Some(total) => format!("{} of {total} achievements", a.unlocked),
                None => format!("{} achievements", a.unlocked),
            });
        }
        parts.join("  ·  ")
    }
    /// A tile took focus: its game is featured, for the stage and for what is fetched ahead.
    pub(super) fn home_focused(&mut self, spot: Spot) {
        if let Spot::Tile(i) = spot {
            if let Some(id) = self.home_games.get(i) {
                self.view.featured = Some(*id);
            }
        }
    }
    /// The row: pinned games, then the recently played, then the most played, each game once and none hidden. A library nobody has
    /// played yet shows its first games. Focus on a tile follows its game wherever the game now sits.
    pub(super) fn rebuild_rows(&mut self) {
        self.rebuild_genres();
        let profile = self.state.profile();
        let mut games: Vec<GameId> = Vec::new();
        // The row reads this TV's history and every service's reports together (see `activity`).
        let lists = [
            activity::favourites(&self.games, &profile.user.favourites),
            activity::recently_played(&self.games, &profile.history),
            activity::most_played(&self.games, &profile.history),
        ];
        for id in lists.into_iter().flatten() {
            if games.len() < TILES && !profile.user.hidden.contains(&id) && !games.contains(&id) {
                games.push(id);
            }
        }
        if games.is_empty() {
            games = self.games.iter().map(|g| g.id).filter(|id| !profile.user.hidden.contains(id)).take(TILES).collect();
        }
        self.home_games = games;
        if matches!(self.view.focus, Focus::Home(Spot::Tile(_) | Spot::Add)) {
            if self.view.featured.is_none_or(|id| !self.home_games.contains(&id)) {
                self.view.featured = self.home_games.first().copied().or(self.view.featured);
            }
            self.view.focus = self.home_focus();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::tests::*;
    #[test]
    fn the_empty_homes_button_opens_accounts() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        m.rebuild_navigation();
        assert_eq!(m.view.focus, Focus::Home(Spot::Add));
        key(&mut m, Key::Ok);
        assert_eq!((m.view.page, m.view.place), (Page::Settings, settings::Place::Accounts));
    }

    #[test]
    fn no_history_and_an_idle_tick_do_not_relayout() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        assert!(m.home_games.is_empty());
        let r = m.revision();
        assert!(m.step(Input::Tick { now_ms: 1000, unix_s: 100 }).is_empty());
        assert_eq!(m.revision(), r);
    }

    #[test]
    fn steam_play_merges_with_this_tvs_history_for_the_row_the_sort_and_the_genre_filter() {
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

        assert_eq!(m.home_games, [beta, gamma, alpha], "Steam's 9000 beats this TV's 7000 beats Steam's 5000");
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
        m.view.filter.show = library::Show::Favourites;
        m.rebuild_filter();
        assert_eq!(m.home_games, [beta, alpha], "both pins lead, and nothing else has been played");
        assert_eq!(m.filtered, [alpha, beta], "the filter takes both, in title order");
        assert_eq!((m.is_favourite(alpha), m.is_favourite(gamma)), (true, false));
        assert_eq!(m.favourite_on(alpha), ["GeForce NOW"]);
        assert!(!m.state().profile().user.favourites.contains(&alpha), "the heart edits only the owner's own set");
    }
}
