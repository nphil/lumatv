//! The Library page: the filter panel on the left (Search, the Show views, the Refine lists), the cover grid beside it, and the same page
//! in search mode, with the keys in place of the panel.
use super::facts::Start;
use super::nav::{Hints, Slice};
use super::*;

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
            Self::Title => "A to Z",
            Self::Recent => "Recently played",
            Self::MostPlayed => "Most played",
            Self::Added => "Recently added",
        }
    }
}
/// Which games the grid shows, one Show row each.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Show {
    #[default]
    All,
    /// The owner's pins, the same set Home leads with, and a service's own favourites.
    Favourites,
    /// First seen in the last 14 days (`facts.added`).
    Added,
    /// Games that can start now (`facts.start`).
    Ready,
    Collection(usize),
    /// The games the owner hid, each with Unhide.
    Hidden,
}
impl Show {
    pub fn name(self, m: &Model) -> String {
        match self {
            Self::All => "All games".into(),
            Self::Favourites => "Favourites".into(),
            Self::Added => "Recently added".into(),
            Self::Ready => "Ready now".into(),
            Self::Collection(i) => m.state.profile().user.collections.get(i).map_or_else(String::new, |c| c.name.clone()),
            Self::Hidden => "Hidden".into(),
        }
    }
}
/// What the Refine rows narrow the view to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub show: Show,
    /// Where the game plays: one service, or anywhere.
    pub plays: Option<EngineId>,
    pub genre: Option<String>,
}
/// One row of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Search,
    Show(Show),
    Plays,
    Genre,
    Sort,
}
/// Where focus last rested on each side of the page, which Left and Right return to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sides {
    pub panel: Option<Focus>,
    pub grid: usize,
}
/// How many games each Show row holds under the Refine choices, counted once per rebuild.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts(pub BTreeMap<Show, usize>);

/// The keys of the search mode: six across.
pub const KEY_COLUMNS: usize = 6;
/// The search keys: 40 characters, then ⌫, Space, Done and the case key.
const KEYS: usize = 44;
/// The rows of search keys.
pub const KEY_ROWS: usize = KEYS.div_ceil(KEY_COLUMNS);

pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        match layer {
            Some(layer) => (0..m.choices(layer).0.len()).map(|i| vec![Focus::SheetRow(i)]).collect(),
            // The panel and the grid are columns of their own heights: `library_links` joins them.
            None => {
                let mut rows: Vec<Vec<Focus>> = if m.view.search {
                    (0..KEYS).step_by(KEY_COLUMNS).map(|i| (i..(i + KEY_COLUMNS).min(KEYS)).map(Focus::Keyboard).collect()).collect()
                } else {
                    (0..m.panel().len()).map(|i| vec![Focus::Filter(i)]).collect()
                };
                if m.games.is_empty() {
                    rows.push(vec![Focus::SheetAction(0)]);
                }
                let cols = m.library_columns();
                rows.extend((0..m.filtered.len()).step_by(cols).map(|i| (i..(i + cols).min(m.filtered.len())).map(Focus::Grid).collect()));
                rows
            }
        }
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        m.library_activate(f, out);
    }
    fn adjust(_m: &mut Model, _f: Focus, _forward: bool, _out: &mut Vec<Command>) -> bool {
        false
    }
    fn default_focus(m: &Model, region: nav::Region) -> Option<Focus> {
        match region {
            nav::Region::Top => Some(Focus::Nav(m.tab())),
            nav::Region::Page if m.view.search => Some(Focus::Keyboard(0)),
            nav::Region::Page if m.games.is_empty() => Some(Focus::SheetAction(0)),
            nav::Region::Page => Some(if m.filtered.is_empty() { Focus::Filter(0) } else { Focus::Grid(0) }),
            nav::Region::Layer => Some(Focus::SheetRow(0)),
        }
    }
    fn step(m: &mut Model, forward: bool, _out: &mut Vec<Command>) -> bool {
        // On the top bar the bumpers step its places.
        if m.view.search || m.view.top().is_some() || matches!(m.view.focus, Focus::Nav(_)) {
            return false;
        }
        let shows: Vec<Show> = m.panel().into_iter().filter_map(|r| if let Row::Show(s) = r { Some(s) } else { None }).collect();
        let at = shows.iter().position(|s| *s == m.view.filter.show).unwrap_or(0);
        let next = if forward { at + 1 } else { at.wrapping_sub(1) };
        let Some(show) = shows.get(next).copied() else { return true };
        m.choose_show(show);
        true
    }
    fn subject(m: &Model) -> Option<GameId> {
        match m.view.focus {
            Focus::Grid(i) => m.filtered.get(i).copied(),
            _ => None,
        }
    }
    fn hints(m: &Model) -> Hints {
        Hints(match (m.view.top(), m.view.focus) {
            (Some(_), _) => vec![(Key::Ok, "Choose"), (Key::Back, "Close")],
            (None, Focus::Grid(_)) if m.view.search => vec![(Key::Ok, "Open"), (Key::Left, "Keys")],
            (None, Focus::Grid(_)) if m.view.filter.show == Show::Hidden => vec![(Key::Ok, "Unhide"), (Key::Left, "Filters")],
            (None, Focus::Grid(_)) => vec![(Key::Ok, "Open"), (Key::Left, "Filters")],
            (None, Focus::Keyboard(_)) => vec![(Key::Ok, "Type"), (Key::Right, "Results"), (Key::Back, "Close")],
            (None, Focus::Filter(_)) => vec![(Key::Ok, "Choose"), (Key::Right, "Games")],
            (None, Focus::SheetAction(_)) => vec![(Key::Ok, "Open"), (Key::Left, "Filters")],
            _ => vec![],
        })
    }
}
impl Model {
    /// The library grid's columns: one fewer than the cover preference gives a full-width row, as the panel takes the left.
    pub fn library_columns(&self) -> usize {
        self.prefs.covers.columns() - 1
    }
    /// The panel's rows, top to bottom: Search; the Show views (collections among them, Hidden while any game is hidden); the Refine rows
    /// (Genre while any game has one).
    pub fn panel(&self) -> Vec<Row> {
        let user = &self.state.profile().user;
        let mut rows = vec![Row::Search];
        rows.extend([Show::All, Show::Favourites, Show::Added, Show::Ready].map(Row::Show));
        rows.extend((0..user.collections.len()).map(|i| Row::Show(Show::Collection(i))));
        if !user.hidden.is_empty() {
            rows.push(Row::Show(Show::Hidden));
        }
        rows.push(Row::Plays);
        if !self.genre_games.is_empty() {
            rows.push(Row::Genre);
        }
        rows.push(Row::Sort);
        rows
    }
    /// The services a game can play on, for "Plays on": every enabled one.
    pub fn plays_on(&self) -> Vec<EngineId> {
        self.integrations.iter().filter(|e| self.enabled(&e.manifest.id)).map(|e| e.manifest.id.clone()).collect()
    }
    /// What "Plays on" calls a service: this TV's own apps are "Native on this TV".
    pub fn plays_name(&self, id: Option<&EngineId>) -> String {
        match id {
            None => "Anywhere".into(),
            Some(id) if self.integration(id).is_some_and(|e| e.manifest.kind == EngineKind::Native) => "Native on this TV".into(),
            Some(id) => self.engine_name(id).into(),
        }
    }
    /// The choices of a Refine list, and which is current.
    pub fn choices(&self, layer: Layer) -> (Vec<String>, usize) {
        match layer {
            Layer::PlaysOn => {
                let engines = self.plays_on();
                let mut names = vec![self.plays_name(None)];
                names.extend(engines.iter().map(|e| self.plays_name(Some(e))));
                let at = self.view.filter.plays.as_ref().and_then(|p| engines.iter().position(|e| e == p)).map_or(0, |i| i + 1);
                (names, at)
            }
            Layer::Genre => {
                let genres = self.genres();
                let at = self.view.filter.genre.as_ref().and_then(|g| genres.iter().position(|n| n == g)).map_or(0, |i| i + 1);
                let mut names = vec!["All genres".to_owned()];
                names.extend(genres);
                (names, at)
            }
            Layer::Sort => (Sort::ALL.iter().map(|s| s.name().into()).collect(), Sort::ALL.iter().position(|s| *s == self.view.sort).unwrap_or(0)),
            _ => (Vec::new(), 0),
        }
    }
    fn choose_show(&mut self, show: Show) {
        self.view.filter.show = show;
        self.view.first_grid_row = 0;
        self.view.library.grid = 0;
        self.rebuild_filter();
        if let Some(i) = self.panel().iter().position(|r| *r == Row::Show(show)) {
            if matches!(self.view.focus, Focus::Filter(_)) {
                self.view.focus = Focus::Filter(i);
            }
        }
    }
    fn library_activate(&mut self, f: Focus, out: &mut Vec<Command>) {
        match f {
            Focus::Grid(i) => {
                let Some(id) = self.filtered.get(i).copied() else { return };
                if self.view.filter.show == Show::Hidden && !self.view.search {
                    self.set_hidden(id, false, out);
                } else {
                    self.open_game(id, None);
                }
            }
            Focus::Filter(i) => match self.panel().get(i).copied() {
                Some(Row::Search) => {
                    self.view.search = true;
                    self.view.focus = Focus::Keyboard(0);
                    self.rebuild_filter();
                }
                Some(Row::Show(show)) => self.choose_show(show),
                Some(Row::Plays) => self.open_refine(Layer::PlaysOn),
                Some(Row::Genre) => self.open_refine(Layer::Genre),
                Some(Row::Sort) => self.open_refine(Layer::Sort),
                None => {}
            },
            Focus::Keyboard(i) => self.keyboard(i, out),
            Focus::SheetAction(_) => self.open_settings(settings::Place::Accounts),
            Focus::SheetRow(i) => {
                match self.view.top() {
                    Some(Layer::PlaysOn) => self.view.filter.plays = i.checked_sub(1).and_then(|i| self.plays_on().get(i).cloned()),
                    Some(Layer::Genre) => self.view.filter.genre = i.checked_sub(1).and_then(|i| self.genres().get(i).cloned()),
                    Some(Layer::Sort) => self.view.sort = Sort::ALL.get(i).copied().unwrap_or_default(),
                    _ => return,
                }
                self.view.first_grid_row = 0;
                self.view.library.grid = 0;
                self.rebuild_filter();
                self.pop_layer();
            }
            _ => {}
        }
    }
    fn open_refine(&mut self, layer: Layer) {
        let at = self.choices(layer).1;
        self.push_layer(layer, Focus::SheetRow(at));
    }
    /// Back in search mode closes the keys: the page stays, on its Search row.
    pub(super) fn leave_search(&mut self) {
        self.view.search = false;
        self.view.query.clear();
        self.view.memory.forget();
        self.view.focus = Focus::Filter(0);
        self.rebuild_filter();
    }
    /// Links the panel (or the keys) and the grid side by side: Left from the grid's first column goes to where focus last rested in the
    /// panel, Right from the panel back to the grid's last focus. Keeps `view.featured` on the focused cover.
    pub(super) fn library_links(&mut self) {
        let cols = self.library_columns();
        let count = self.filtered.len();
        match self.view.focus {
            Focus::Grid(i) => {
                self.view.library.grid = i;
                self.view.featured = self.filtered.get(i).copied().or(self.view.featured);
            }
            f @ (Focus::Filter(_) | Focus::Keyboard(_)) => self.view.library.panel = Some(f),
            _ => {}
        }
        let search = self.view.search;
        let panel_len = if search { KEYS } else { self.panel().len() };
        let panel_back = self.view.library.panel.filter(|f| match f {
            Focus::Keyboard(_) => search,
            Focus::Filter(i) => !search && *i < panel_len,
            _ => false,
        });
        let panel_back = panel_back.unwrap_or(if search { Focus::Keyboard(0) } else { Focus::Filter(1) });
        let grid_back = if count > 0 {
            Some(Focus::Grid(self.view.library.grid.min(count - 1)))
        } else {
            self.games.is_empty().then_some(Focus::SheetAction(0))
        };
        let top = Focus::Nav(self.tab());
        for n in &mut self.navigation {
            match n.id {
                Focus::Nav(_) => n.down = grid_back.or(Some(panel_back)),
                Focus::Filter(i) => {
                    n.left = None;
                    n.right = grid_back;
                    n.up = Some(if i == 0 { top } else { Focus::Filter(i - 1) });
                    n.down = (i + 1 < panel_len).then_some(Focus::Filter(i + 1));
                }
                Focus::Keyboard(i) => {
                    let column = i % KEY_COLUMNS;
                    n.left = (column > 0).then(|| Focus::Keyboard(i - 1));
                    n.right = if column + 1 < KEY_COLUMNS && i + 1 < KEYS { Some(Focus::Keyboard(i + 1)) } else { grid_back };
                    n.up = Some(if i < KEY_COLUMNS { top } else { Focus::Keyboard(i - KEY_COLUMNS) });
                    n.down = (i - column + KEY_COLUMNS < KEYS).then(|| Focus::Keyboard((i + KEY_COLUMNS).min(KEYS - 1)));
                }
                Focus::SheetAction(_) => {
                    (n.left, n.right, n.up, n.down) = (Some(panel_back), None, Some(top), None);
                }
                Focus::Grid(i) => {
                    let column = i % cols;
                    n.left = Some(if column == 0 { panel_back } else { Focus::Grid(i - 1) });
                    n.right = (column + 1 < cols && i + 1 < count).then(|| Focus::Grid(i + 1));
                    n.up = Some(if i < cols { top } else { Focus::Grid(i - cols) });
                    n.down = (i - column + cols < count).then(|| Focus::Grid((i + cols).min(count - 1)));
                }
                _ => {}
            }
        }
    }
    /// Whether `game` passes the Refine choices.
    fn refined(&self, game: &Game) -> bool {
        let f = &self.view.filter;
        f.plays.as_ref().is_none_or(|id| game.sources.iter().any(|s| &s.engine == id))
            && f.genre.as_ref().is_none_or(|genre| self.genre_games.get(genre).is_some_and(|ids| ids.contains(&game.id)))
    }
    fn in_show(&self, game: &Game, show: Show) -> bool {
        let user = &self.state.profile().user;
        let hidden = user.hidden.contains(&game.id);
        match show {
            Show::Hidden => hidden,
            _ if hidden => false,
            Show::All => true,
            Show::Favourites => activity::is_favourite(game, &user.favourites),
            Show::Added => self.added(game.id).is_some(),
            Show::Ready => self.can_start(game.id) == Start::Ready,
            Show::Collection(i) => user.collections.get(i).is_some_and(|c| c.games.contains(&game.id)),
        }
    }
    pub(crate) fn rebuild_filter(&mut self) {
        let shows: Vec<Show> = self.panel().into_iter().filter_map(|r| if let Row::Show(s) = r { Some(s) } else { None }).collect();
        if !shows.contains(&self.view.filter.show) {
            self.view.filter.show = Show::All;
        }
        let mut counts = BTreeMap::new();
        for show in &shows {
            counts.insert(*show, self.games.iter().filter(|g| self.refined(g) && self.in_show(g, *show)).count());
        }
        self.library_counts = Counts(counts);
        let p = self.state.profile();
        let query = self.view.query.to_lowercase();
        let mut shown: Vec<&Game> = if self.view.search {
            self.games.iter().filter(|g| !p.user.hidden.contains(&g.id) && g.title.to_lowercase().contains(&query)).collect()
        } else {
            self.games.iter().filter(|g| self.refined(g) && self.in_show(g, self.view.filter.show)).collect()
        };
        // A stable sort: games with equal keys, and the ones nothing was played of, stay in title order.
        match self.view.sort {
            Sort::Title => {}
            Sort::Recent => shown.sort_by_key(|g| std::cmp::Reverse(activity::last_played(g, &p.history).map_or(0, |last| last.at))),
            Sort::MostPlayed => shown.sort_by_key(|g| std::cmp::Reverse(activity::play_seconds(g, &p.history))),
            Sort::Added => shown.sort_by_key(|g| std::cmp::Reverse((p.user.first_seen.get(&g.id).copied(), g.id.0))),
        }
        self.filtered = shown.into_iter().map(|g| g.id).collect();
        self.view.first_grid_row = self.view.first_grid_row.min(self.filtered.len().saturating_sub(1) / self.library_columns());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::tests::*;

    /// The library page with six GeForce NOW games and an empty collection.
    fn library() -> Model {
        use lumagame_core::activity::Activity;
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud)]);
        describe(&mut m, "gfn", vec![], &[]);
        engine_says(&mut m, "gfn", ipc::Event::Targets { targets: vec![linked("main")] });
        let entries = ["Alpha", "Bravo", "Charlie", "Delta", "Echo", "Foxtrot"]
            .iter()
            .enumerate()
            .map(|(i, t)| entry(&format!("{i}"), t, ipc::Role::Game, &[], Activity::default()))
            .collect();
        engine_says(&mut m, "gfn", ipc::Event::Library { target: TargetId::new("main").unwrap(), entries, next: None });
        m.state.profile_mut().user.collections.push(lumagame_core::state::Collection { name: "Couch".into(), games: vec![] });
        m.switch_tab(1, Focus::Nav(1));
        m.rebuild_filter();
        m.rebuild_navigation();
        m
    }
    fn row(m: &Model, wanted: Row) -> Focus {
        Focus::Filter(m.panel().iter().position(|r| *r == wanted).unwrap())
    }
    fn press(m: &mut Model, f: Focus) {
        m.set_focus(f);
        m.rebuild_navigation();
        key(m, Key::Ok);
    }

    #[test]
    fn a_show_row_changes_the_grid_and_its_count_and_the_bumpers_step_the_rows() {
        let mut m = library();
        let all = m.filtered.len();
        assert_eq!(m.library_counts.0[&Show::All], all);
        let first = m.filtered[0];
        m.state.profile_mut().user.collections[0].games.push(first);
        m.rebuild_filter();
        assert_eq!(m.library_counts.0[&Show::Collection(0)], 1);
        let f = row(&m, Row::Show(Show::Collection(0)));
        press(&mut m, f);
        assert_eq!(m.filtered, [first]);
        key(&mut m, Key::Prev);
        assert_eq!(m.view.filter.show, Show::Ready);
        key(&mut m, Key::Prev);
        key(&mut m, Key::Prev);
        key(&mut m, Key::Prev);
        assert_eq!((m.view.filter.show, m.filtered.len()), (Show::All, all));
        key(&mut m, Key::Prev);
        assert_eq!(m.view.filter.show, Show::All, "nothing wraps");
    }

    #[test]
    fn refine_choices_compose_with_the_show_view() {
        let mut m = library();
        let first = m.filtered[0];
        m.state.profile_mut().user.favourites.insert(first);
        let f = row(&m, Row::Show(Show::Favourites));
        press(&mut m, f);
        let f = row(&m, Row::Plays);
        press(&mut m, f);
        assert_eq!(m.view.top(), Some(Layer::PlaysOn));
        assert_eq!(m.choices(Layer::PlaysOn).0[0], "Anywhere");
        key(&mut m, Key::Down);
        key(&mut m, Key::Ok);
        assert_eq!((m.view.top(), m.filtered.clone()), (None, vec![first]), "a service that has the favourite keeps it");
        m.view.filter.plays = Some(EngineId::new("elsewhere").unwrap());
        m.rebuild_filter();
        assert!(m.filtered.is_empty() && m.library_counts.0[&Show::All] == 0, "and the counts follow the refine");
    }

    #[test]
    fn left_and_right_return_where_focus_last_rested_on_each_side() {
        let mut m = library();
        m.set_focus(Focus::Grid(0));
        m.rebuild_navigation();
        key(&mut m, Key::Left);
        assert_eq!(m.view.focus, Focus::Filter(1), "first time in: All games");
        key(&mut m, Key::Down);
        key(&mut m, Key::Right);
        assert_eq!(m.view.focus, Focus::Grid(0));
        key(&mut m, Key::Down);
        assert_eq!(m.view.focus, Focus::Grid(m.library_columns()));
        key(&mut m, Key::Left);
        assert_eq!(m.view.focus, Focus::Filter(2), "the panel resumes");
        key(&mut m, Key::Right);
        assert_eq!(m.view.focus, Focus::Grid(m.library_columns()), "and so does the grid");
    }

    #[test]
    fn search_narrows_per_key_and_back_leaves_the_keys_before_the_page() {
        let mut m = library();
        press(&mut m, Focus::Filter(0));
        assert!(m.view.search && m.view.focus == Focus::Keyboard(0));
        let before = m.filtered.len();
        key(&mut m, Key::Ok);
        assert_eq!(m.view.query, "A");
        assert!(m.filtered.len() <= before && m.filtered.iter().all(|id| m.game(*id).unwrap().title.to_lowercase().contains('a')));
        m.view.query = "zzz".into();
        m.rebuild_filter();
        assert!(m.filtered.is_empty(), "no results");
        key(&mut m, Key::Back);
        assert_eq!((m.view.page, m.view.search, m.view.focus), (Page::Library, false, Focus::Filter(0)));
        m.switch_tab(2, Focus::Keyboard(0));
        m.rebuild_navigation();
        key(&mut m, Key::Back);
        assert_eq!((m.view.page, m.view.search), (Page::Library, false), "from the top bar's Search too");
    }

    #[test]
    fn unhide_returns_a_game_and_hidden_leaves_the_panel_with_the_last() {
        let mut m = library();
        let first = m.filtered[0];
        m.set_hidden(first, true, &mut Vec::new());
        assert!(!m.filtered.contains(&first));
        let f = row(&m, Row::Show(Show::Hidden));
        press(&mut m, f);
        m.set_focus(Focus::Grid(0));
        assert_eq!(m.hints().0[0], (Key::Ok, "Unhide"));
        assert_eq!(m.hints().0[0], (Key::Ok, "Unhide"));
        press(&mut m, Focus::Grid(0));
        assert!(!m.panel().contains(&Row::Show(Show::Hidden)));
        assert_eq!(m.view.filter.show, Show::All);
        assert!(m.filtered.contains(&first));
    }

    #[test]
    fn an_empty_library_offers_accounts_and_ok_on_a_cover_opens_its_page() {
        let mut m = Model::new(State::default(), Listing::default(), vec![]);
        m.switch_tab(1, Focus::Nav(1));
        m.rebuild_navigation();
        assert!(m.filtered.is_empty());
        assert_eq!(Rules::default_focus(&m, nav::Region::Page), Some(Focus::SheetAction(0)));
        press(&mut m, Focus::SheetAction(0));
        assert_eq!((m.view.page, m.view.place), (Page::Settings, settings::Place::Accounts), "the one button adds accounts and PCs");
        let mut m = library();
        press(&mut m, Focus::Grid(0));
        assert_eq!((m.view.page, m.view.selected), (Page::Game, Some(id_of(&library(), "Alpha"))));
    }
}
