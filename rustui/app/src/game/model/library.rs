//! The Library and Search pages: the filter bar, the sort and genre choices, the cover grid and the search keyboard.
use super::nav::Slice;
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
pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        m.library_grid(layer)
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        m.library_activate(f, out);
    }
    fn adjust(_m: &mut Model, _f: Focus, _forward: bool, _out: &mut Vec<Command>) -> bool {
        false
    }
}
impl Model {
    fn library_grid(&self, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        let mut rows: Vec<Vec<Focus>> = Vec::new();
        match layer {
            Some(Layer::Sort) => rows.extend((0..4).map(|i| vec![Focus::SheetRow(i)])),
            Some(Layer::Genre) => rows.extend((0..=self.genres().len()).map(|i| vec![Focus::SheetRow(i)])),
            Some(_) => {}
            None => {
                if self.view.page == Page::Library {
                    rows.push((0..self.integrations.iter().filter(|e| self.enabled(&e.manifest.id)).count() + 5).map(Focus::Filter).collect());
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
        }
        rows
    }
    fn library_activate(&mut self, f: Focus, out: &mut Vec<Command>) {
        match f {
            Focus::Grid(i) => {
                if let Some(id) = self.filtered.get(i).copied() {
                    self.open_game(id, None);
                }
            }
            Focus::Filter(i) => {
                let engines = self.integrations.iter().filter(|e| self.enabled(&e.manifest.id)).map(|e| e.manifest.id.clone()).collect::<Vec<_>>();
                if i == 0 {
                    self.view.filter = Filter::default();
                } else if let Some(id) = engines.get(i - 1) {
                    self.view.filter.engine = Some(id.clone());
                } else if i == engines.len() + 1 {
                    self.push_layer(Layer::Genre, Focus::SheetRow(0));
                } else if i == engines.len() + 2 {
                    self.view.filter.favourites = !self.view.filter.favourites;
                } else if i == engines.len() + 3 {
                    self.push_layer(Layer::Sort, Focus::SheetRow(Sort::ALL.iter().position(|s| *s == self.view.sort).unwrap_or(0)));
                } else {
                    self.refresh_libraries(None, out);
                    return;
                }
                self.view.first_grid_row = 0;
                self.rebuild_filter();
            }
            Focus::SearchField => self.view.focus = Focus::Keyboard(0),
            Focus::Keyboard(i) => self.keyboard(i, out),
            Focus::SheetRow(i) if self.view.top() == Some(Layer::Sort) => {
                if let Some(sort) = Sort::ALL.get(i) {
                    self.view.sort = *sort;
                }
                self.rebuild_filter();
                self.pop_layer();
            }
            Focus::SheetRow(i) if self.view.top() == Some(Layer::Genre) => {
                self.view.filter.genre = if i == 0 { None } else { self.genres().get(i - 1).cloned() };
                self.rebuild_filter();
                self.pop_layer();
            }
            _ => {}
        }
    }
    pub fn filter_count(&self, engine: Option<&EngineId>) -> usize {
        self.games
            .iter()
            .filter(|g| !self.state.profile().user.hidden.contains(&g.id) && engine.is_none_or(|e| g.sources.iter().any(|s| &s.engine == e)))
            .count()
    }
    pub(super) fn rebuild_filter(&mut self) {
        let p = self.state.profile();
        let f = &self.view.filter;
        let query = self.view.query.to_lowercase();
        let mut shown: Vec<&Game> = self
            .games
            .iter()
            .filter(|g| {
                !p.user.hidden.contains(&g.id)
                    && (!self.view.search || g.title.to_lowercase().contains(&query))
                    && f.engine.as_ref().is_none_or(|id| g.sources.iter().any(|s| &s.engine == id))
                    && (!f.favourites || activity::is_favourite(g, &p.user.favourites))
                    && f.collection.is_none_or(|i| p.user.collections.get(i).is_some_and(|c| c.games.contains(&g.id)))
                    && f.genre.as_ref().is_none_or(|genre| self.genre_games.get(genre).is_some_and(|ids| ids.contains(&g.id)))
            })
            .collect();
        // A stable sort: games with equal keys, and the ones nothing was played of, stay in title order.
        match self.view.sort {
            Sort::Title => {}
            Sort::Recent => shown.sort_by_key(|g| std::cmp::Reverse(activity::last_played(g, &p.history).map_or(0, |last| last.at))),
            Sort::MostPlayed => shown.sort_by_key(|g| std::cmp::Reverse(activity::play_seconds(g, &p.history))),
            Sort::Added => shown.sort_by_key(|g| std::cmp::Reverse(g.id.0)),
        }
        self.filtered = shown.into_iter().map(|g| g.id).collect();
        self.view.first_grid_row = self.view.first_grid_row.min(self.filtered.len().saturating_sub(1) / self.prefs.covers.columns());
    }
}
