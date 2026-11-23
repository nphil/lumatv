//! The game's page: Play, where to play it, and four cards that each open their section over the artwork; the reader of a post over
//! a section; the chooser of ways to play; and what the page knows beyond its artwork (the owner's own achievements from an engine
//! that can read them, and the quiet look-ahead that fetches a game's page data while its tile rests in focus).
pub mod media;

use super::facts::{Ask, Fact, Reader, Reason, Section, TopicId, steam_app, why};
use super::nav::{Hints, Slice};
use super::settings::Place;
use super::*;
use lumagame_core::launch::{self, Candidate, Resolution};
use lumagame_meta::hub::{Deck, Hub};
use lumagame_core::ids::ProviderGameId;

/// How long a tile must stay focused before its page data is fetched ahead of the owner opening it.
pub(super) const REST_MS: u64 = 700;

/// The page's cards and the sections they open, in order.
pub const SECTIONS: [Section; 4] = [Section::Activity, Section::Community, Section::Media, Section::About];
/// A section's name on its card and its tab.
pub fn section_name(section: Section) -> &'static str {
    match section {
        Section::Activity => "Activity",
        Section::Community => "Community",
        Section::Media => "Media",
        Section::About => "About",
    }
}
pub fn section_index(section: Section) -> usize {
    SECTIONS.iter().position(|s| *s == section).unwrap_or(0)
}

/// A reader line holds about this many characters at 100 % text, the measure long text reads best at; larger text holds fewer.
const READER_CHARS: f32 = 60.0;
/// At 100 % text: the room the reader's body and head share, its head (crumb, title, byline, foot) and one body line, in pixels. All
/// three grow with the text, so a page holds 13 lines at 100 % and 4 at 200 %.
const READER_ROOM: f32 = 760.0;
const READER_HEAD: f32 = 224.0;
const READER_LINE: f32 = 39.0;

/// What the game page keeps between keys: the row each section last had in focus, in [`SECTIONS`] order, and what the reader shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Glance {
    pub rows: [usize; 4],
    pub reading: Option<Reading>,
}
/// What the reader shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Reading {
    News(usize),
    Review(usize),
    Thread(TopicId),
    Guide(usize),
    About,
}
/// A row of a section that OK acts on.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// The engine that reads the owner's achievements is not signed in.
    SignIn { engine: EngineId, expired: bool },
    News(usize),
    Review(usize),
    Thread(usize),
    Guide(usize),
    About,
}
/// The reader's text, its body in pages of lines.
#[derive(Clone, Debug, PartialEq)]
pub struct ReaderText {
    pub title: String,
    pub byline: String,
    pub pages: Vec<Vec<String>>,
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
        Self {
            description: hub.description.clone(),
            year,
            genres: hub.genres.clone(),
            players: (!players.is_empty()).then(|| players.join(", ")),
            developer: (!hub.developers.is_empty()).then(|| hub.developers.join(", ")),
            capabilities,
            cloud_saves: hub.categories.iter().any(|c| c == "Steam Cloud").then_some(true),
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
/// `text` in lines of at most `width` characters, broken between words; a blank line between paragraphs stays.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        if !line.is_empty() || lines.last().is_some_and(|l| !l.is_empty()) {
            lines.push(line);
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}
pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        m.game_grid(layer)
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        m.game_activate(f, out);
    }
    /// Left and Right over a section switch sections; in Media they first move along the filter or a row of stills.
    fn adjust(m: &mut Model, f: Focus, forward: bool, _out: &mut Vec<Command>) -> bool {
        match m.media_sideways(f, forward) {
            Some(moved) => moved,
            None => m.switch_section(forward),
        }
    }
    fn anchor(_m: &Model) -> Option<Focus> {
        Some(Focus::Play)
    }
    fn step(m: &mut Model, forward: bool, _out: &mut Vec<Command>) -> bool {
        m.switch_section(forward)
    }
    fn subject(m: &Model) -> Option<GameId> {
        m.view.selected
    }
    fn hints(m: &Model) -> Hints {
        Hints(match m.view.top() {
            Some(Layer::Section(_)) => vec![(Key::Ok, m.media_ok().unwrap_or("Open")), (Key::Next, "Section"), (Key::Back, "Close")],
            Some(Layer::Reader) => vec![(Key::Down, "Next page"), (Key::Back, "Close")],
            Some(Layer::Chooser) => vec![(Key::Ok, "Play"), (Key::Back, "Close")],
            _ => vec![(Key::Down, "More"), (Key::Menu, "Menu")],
        })
    }
}

impl Model {
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
        match layer {
            Some(Layer::Chooser) => {
                let ways = self.selected_game().map_or(0, |g| g.sources.len());
                let mut rows: Vec<Vec<Focus>> = (0..ways).map(|i| vec![Focus::SheetRow(i)]).collect();
                rows.push(vec![Focus::Remember]);
                rows
            }
            Some(Layer::Section(Section::Media)) => self.media_grid(),
            Some(Layer::Section(section)) => self.section_rows(section).into_iter().map(|f| vec![f]).collect(),
            Some(Layer::Reader) => (0..self.reader_text().map_or(0, |r| r.pages.len())).map(|i| vec![Focus::ReaderPage(i)]).collect(),
            Some(_) => Vec::new(),
            None => {
                let mut rows = vec![vec![Focus::Play, Focus::Where]];
                // Larger text lays the cards out two by two.
                let per_row = if self.prefs.text_percent > 100 { 2 } else { SECTIONS.len() };
                rows.extend((0..SECTIONS.len()).collect::<Vec<_>>().chunks(per_row).map(|row| row.iter().map(|&i| Focus::Card(i)).collect()));
                rows
            }
        }
    }
    fn game_activate(&mut self, f: Focus, out: &mut Vec<Command>) {
        match f {
            Focus::Play => {
                if let Some(id) = self.view.selected {
                    self.play(id, out);
                }
            }
            Focus::Where => self.open_chooser(),
            Focus::Card(i) => {
                if let Some(&section) = SECTIONS.get(i) {
                    self.open_section(section);
                }
            }
            Focus::Entry(i) => self.open_entry(i, out),
            Focus::Media(i) => self.open_media(i, out),
            Focus::SheetRow(i) if self.view.top() == Some(Layer::Chooser) => self.choose(i, out),
            Focus::Remember => self.view.remember = !self.view.remember,
            _ => {}
        }
    }

    /// The rows of a section that OK acts on. The sign-in row leads Activity while the engine that reads the owner's achievements is
    /// signed out; without such an engine there is none.
    pub fn section_entries(&self, id: GameId, section: Section) -> Vec<Entry> {
        fn count<T>(fact: &Fact<&[T]>) -> usize {
            if let Fact::Present(items) = fact { items.len() } else { 0 }
        }
        let facts = self.facts(id);
        let mut entries = Vec::new();
        match section {
            Section::Activity => {
                if let Fact::SignIn { expired } = facts.activity.achievements {
                    if let Some(engine) = self.sign_in_engine() {
                        entries.push(Entry::SignIn { engine: engine.clone(), expired });
                    }
                }
                entries.extend((0..count(&facts.activity.news)).map(Entry::News));
            }
            Section::Community => {
                let reviews = if let Fact::Present(r) = facts.community.reviews { r.top.len() } else { 0 };
                entries.extend((0..reviews).map(Entry::Review));
                entries.extend((0..count(&facts.community.discussions)).map(Entry::Thread));
                entries.extend((0..count(&facts.community.guides)).map(Entry::Guide));
            }
            // Its rows are the Media slice's filter and stills (`media_focuses`).
            Section::Media => {}
            Section::About => {
                if matches!(facts.about.description, Fact::Present(_)) {
                    entries.push(Entry::About);
                }
            }
        }
        entries
    }
    /// The enabled engine that would read the owner's achievements once signed in.
    fn sign_in_engine(&self) -> Option<&EngineId> {
        self.integrations
            .iter()
            .find(|e| {
                self.enabled(&e.manifest.id)
                    && e.descriptor.as_ref().is_some_and(|d| d.capabilities.has(Feature::Achievements))
                    && !e.targets.iter().any(|t| t.link == ipc::Link::Linked)
            })
            .map(|e| &e.manifest.id)
    }
    fn section_rows(&self, section: Section) -> Vec<Focus> {
        let Some(id) = self.view.selected else { return Vec::new() };
        if section == Section::Media {
            return self.media_focuses();
        }
        (0..self.section_entries(id, section).len()).map(Focus::Entry).collect()
    }
    /// Where focus lands in a section: the row it last rested on, else the last row there is.
    fn section_focus(&self, section: Section) -> Focus {
        let rows = self.section_rows(section);
        rows.get(self.view.glance.rows[section_index(section)]).or(rows.last()).copied().unwrap_or(Focus::None)
    }
    fn open_section(&mut self, section: Section) {
        let focus = self.section_focus(section);
        self.push_layer(Layer::Section(section), focus);
    }
    /// Over a section, the next or previous section takes its place, at the row it last had in focus. Back still returns to the card
    /// the layer was opened from.
    fn switch_section(&mut self, forward: bool) -> bool {
        let Some(Layer::Section(section)) = self.view.top() else { return false };
        let at = section_index(section);
        let next = if forward { at + 1 } else { at.wrapping_sub(1) };
        let Some(&next) = SECTIONS.get(next) else { return true };
        if let Some(row) = self.section_rows(section).iter().position(|f| *f == self.view.focus) {
            self.view.glance.rows[at] = row;
        }
        if let Some(top) = self.view.layers.last_mut() {
            *top = Layer::Section(next);
        }
        self.view.focus = self.section_focus(next);
        true
    }
    fn open_entry(&mut self, i: usize, out: &mut Vec<Command>) {
        let (Some(id), Some(Layer::Section(section))) = (self.view.selected, self.view.top()) else { return };
        let Some(entry) = self.section_entries(id, section).into_iter().nth(i) else { return };
        let reading = match entry {
            Entry::SignIn { engine, .. } => return self.begin_auth(engine, out),
            Entry::News(i) => Reading::News(i),
            Entry::Review(i) => Reading::Review(i),
            Entry::Guide(i) => Reading::Guide(i),
            Entry::About => Reading::About,
            Entry::Thread(i) => {
                let topic = match self.facts(id).community.discussions {
                    Fact::Present(topics) => topics.get(i).map(TopicId::of),
                    _ => None,
                };
                let Some(topic) = topic else { return };
                self.open_thread(id, topic.clone(), out);
                Reading::Thread(topic)
            }
        };
        self.view.glance.reading = Some(reading);
        self.push_layer(Layer::Reader, Focus::ReaderPage(0));
    }
    /// What the reader shows, its body broken into lines of about sixty characters at 100 % text and into pages that fit the layer.
    pub fn reader_text(&self) -> Option<ReaderText> {
        let id = self.view.selected?;
        let facts = self.facts(id);
        let (title, byline, body) = match self.view.glance.reading.as_ref()? {
            Reading::News(i) => {
                let Fact::Present(news) = facts.activity.news else { return None };
                let n = news.get(*i)?;
                let when = self.ago(n.at);
                (n.title.clone(), n.author.as_ref().map_or(when.clone(), |a| format!("{a}, {when}")), n.text.clone())
            }
            Reading::Review(i) => {
                let Fact::Present(reviews) = facts.community.reviews else { return None };
                let r = reviews.top.get(*i)?;
                let verdict = if r.recommended { "Recommended" } else { "Not recommended" };
                (verdict.into(), format!("{} on record, {} found it helpful", play_time(r.playtime_s), r.helpful), r.text.clone())
            }
            Reading::Guide(i) => {
                let Fact::Present(guides) = facts.community.guides else { return None };
                let g = guides.get(*i)?;
                (g.title.clone(), g.author.clone(), g.summary.clone())
            }
            Reading::Thread(topic) => {
                let named = match facts.community.discussions {
                    Fact::Present(topics) => topics.iter().find(|t| TopicId::of(t) == *topic).map(|t| t.title.clone()),
                    _ => None,
                };
                match self.thread(id, topic) {
                    Fact::Present(thread) => {
                        let posts: Vec<String> = thread.posts.iter().map(|p| format!("{}: {}", p.author, p.text)).collect();
                        (thread.title.clone(), format!("{} replies", thread.replies), posts.join("\n\n"))
                    }
                    Fact::Loading => (named.unwrap_or_default(), String::new(), "Loading".into()),
                    Fact::Absent(reason) => (named.unwrap_or_default(), String::new(), why(reason).into()),
                    Fact::SignIn { .. } => (named.unwrap_or_default(), String::new(), why(Reason::Other).into()),
                }
            }
            Reading::About => {
                let Fact::Present(description) = facts.about.description else { return None };
                let developer = if let Fact::Present(d) = facts.about.developer { d.to_owned() } else { String::new() };
                (self.game(id).map(|g| g.title.clone()).unwrap_or_default(), developer, description.to_owned())
            }
        };
        let scale = f32::from(self.prefs.text_percent) / 100.0;
        let lines = wrap(&body, (READER_CHARS / scale) as usize);
        let per_page = (((READER_ROOM - READER_HEAD * scale) / (READER_LINE * scale)) as usize).max(2);
        let mut pages: Vec<Vec<String>> = lines.chunks(per_page).map(<[String]>::to_vec).collect();
        if pages.is_empty() {
            pages.push(Vec::new());
        }
        Some(ReaderText { title, byline, pages })
    }
    /// How long ago a dated post was, by the day.
    pub fn ago(&self, at: u64) -> String {
        match self.unix_s.saturating_sub(at) / 86_400 {
            0 => "Today".into(),
            1 => "Yesterday".into(),
            days => format!("{days} days ago"),
        }
    }
    /// The pictures the page and its open section show: the latest achievements' icons and the first store screenshots on the cards,
    /// and what the Media section or the viewer shows while it is open.
    pub(super) fn glance_art(&self, id: GameId) -> Vec<String> {
        let facts = self.facts(id);
        let mut urls = Vec::new();
        if let Fact::Present(a) = &facts.activity.achievements {
            urls.extend(a.latest.iter().take(3).filter_map(|a| a.icon.map(str::to_owned)));
        }
        if let Fact::Present(shots) = facts.media.store_screenshots {
            urls.extend(shots.iter().take(4).map(|s| s.thumb.clone()));
        }
        urls.extend(self.media_art(id));
        urls
    }
    /// A way to play from the chooser: one that needs a sign-in or a pairing starts it; one that can start plays, and Remember pins
    /// it to the game (or, off, forgets a pinned way).
    fn choose(&mut self, i: usize, out: &mut Vec<Command>) {
        let (Some(game), Some(source)) = (self.view.selected, self.selected_game().and_then(|g| g.sources.get(i)).map(|s| s.key())) else { return };
        match self.readiness(&source) {
            Readiness::SignIn => self.begin_auth(source.engine.clone(), out),
            ready if ready.can_attempt() => {
                let remember = self.view.remember;
                let pinned = &mut self.state.profile_mut().user.pinned;
                if remember {
                    pinned.insert(game, source.clone());
                } else {
                    pinned.remove(&game);
                }
                out.push(Command::SaveState);
                self.view.chooser_selected = Some(source.clone());
                self.start(game, source, false, out);
            }
            _ => self.view.notice = Some("This way to play is unavailable. Choose another.".into()),
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
    /// Opens the game's page, with `section` open over it when given.
    pub fn open_game(&mut self, id: GameId, section: Option<Section>) {
        self.view.selected = Some(id);
        self.view.chooser_selected = None;
        self.view.glance = Glance::default();
        self.view.notice = None;
        self.show_page(Page::Game, Focus::Play);
        if let Some(section) = section {
            self.open_section(section);
        }
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
    use lumagame_core::{activity::Activity, community::{Community, NewsItem, Section as Part, Shot}, engine::AuthMethod, ids::ProviderGameId};

    /// GeForce NOW lists Hades II (Steam app 1145350) and Steam, on with achievements and the community, has said two long news posts.
    /// Steam is signed in when `signed_in`.
    fn hades(signed_in: bool) -> (Model, GameId) {
        let news = (0..2).map(|i| NewsItem { title: format!("Patch {i}"), text: "word ".repeat(300), author: None, source: None, at: 0 }).collect();
        hades_with(signed_in, Community { news: Part::list(news), ..Community::default() })
    }
    /// Hades II as `hades` has it, with `community` for what Steam's community says of it.
    pub(super) fn hades_with(signed_in: bool, community: Community) -> (Model, GameId) {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud), manifest("steam", "Steam", EngineKind::Account)]);
        describe(&mut m, "gfn", vec![AuthMethod::DeviceCode], &[]);
        describe(&mut m, "steam", vec![AuthMethod::Qr], &[Feature::Achievements, Feature::Community]);
        engine_says(&mut m, "gfn", ipc::Event::Targets { targets: vec![linked("main")] });
        if signed_in {
            engine_says(&mut m, "steam", ipc::Event::Targets { targets: vec![linked("1")] });
        }
        let entries = vec![entry("1", "Hades II", ipc::Role::Game, &["steam:1145350"], Activity::default())];
        engine_says(&mut m, "gfn", ipc::Event::Library { target: TargetId::new("main").unwrap(), entries, next: None });
        engine_says(&mut m, "steam", ipc::Event::Community { game: ProviderGameId::new("1145350").unwrap(), community });
        let id = id_of(&m, "Hades II");
        m.open_game(id, None);
        m.rebuild_navigation();
        (m, id)
    }
    #[test]
    fn ok_on_a_card_opens_its_section_and_back_returns_to_the_card_then_play_then_leaves() {
        let (mut m, _) = hades(true);
        m.set_focus(Focus::Card(1));
        key(&mut m, Key::Ok);
        assert_eq!(m.view.top(), Some(Layer::Section(Section::Community)));
        key(&mut m, Key::Back);
        assert_eq!((m.view.top(), m.view.focus), (None, Focus::Card(1)));
        key(&mut m, Key::Back);
        assert_eq!((m.view.page, m.view.focus), (Page::Game, Focus::Play));
        key(&mut m, Key::Back);
        assert_ne!(m.view.page, Page::Game);
    }
    #[test]
    fn each_section_keeps_its_own_focus_across_switches() {
        let shots = (0..9).map(|i| Shot { thumb: format!("https://x/{i}.jpg"), full: format!("https://x/{i}.jpg") }).collect();
        let (mut m, id) = hades_with(true, Community { store_screenshots: Part::list(shots), ..Community::default() });
        let hub = Hub { description: "A god's daughter.".into(), ..Hub::default() };
        m.step(Input::Details { game: id, details: Box::new(GameDetails::from_hub(hub)) });
        m.set_focus(Focus::Card(2));
        key(&mut m, Key::Ok);
        key(&mut m, Key::Down);
        key(&mut m, Key::Down);
        assert_eq!(m.view.focus, Focus::Media(6));
        key(&mut m, Key::Next);
        assert_eq!((m.view.top(), m.view.focus), (Some(Layer::Section(Section::About)), Focus::Entry(0)), "the bumper steps to About");
        key(&mut m, Key::Left);
        assert_eq!((m.view.top(), m.view.focus), (Some(Layer::Section(Section::Media)), Focus::Media(6)), "Left comes back to the same picture");
    }
    #[test]
    fn a_news_post_opens_the_reader_and_down_turns_its_page_and_back_closes_only_the_reader() {
        let (mut m, _) = hades(true);
        m.set_focus(Focus::Card(0));
        key(&mut m, Key::Ok);
        assert_eq!(m.view.focus, Focus::Entry(0));
        key(&mut m, Key::Ok);
        let text = m.reader_text().unwrap();
        assert_eq!((m.view.top(), text.title.as_str()), (Some(Layer::Reader), "Patch 0"));
        assert!(text.pages.len() > 1 && text.pages[0].iter().all(|l| l.chars().count() <= 60));
        key(&mut m, Key::Down);
        assert_eq!(m.view.focus, Focus::ReaderPage(1));
        key(&mut m, Key::Back);
        assert_eq!((m.view.top(), m.view.focus), (Some(Layer::Section(Section::Activity)), Focus::Entry(0)));
    }
    #[test]
    fn signed_out_steam_leads_activity_with_a_sign_in_row_and_none_shows_with_steam_off() {
        let (mut m, id) = hades(false);
        assert_eq!(m.section_entries(id, Section::Activity)[0], Entry::SignIn { engine: EngineId::new("steam").unwrap(), expired: false });
        m.set_focus(Focus::Card(0));
        key(&mut m, Key::Ok);
        key(&mut m, Key::Ok);
        assert_eq!(m.view.top(), Some(Layer::Auth), "OK begins the sign-in");
        m.state.engines.get_mut(&EngineId::new("steam").unwrap()).unwrap().enabled = false;
        assert!(!m.section_entries(id, Section::Activity).iter().any(|e| matches!(e, Entry::SignIn { .. })));
    }
    #[test]
    fn remember_in_the_chooser_pins_the_way_chosen() {
        let (mut m, id) = hades(true);
        m.set_focus(Focus::Where);
        key(&mut m, Key::Ok);
        assert_eq!(m.view.top(), Some(Layer::Chooser));
        m.set_focus(Focus::Remember);
        key(&mut m, Key::Ok);
        m.set_focus(Focus::SheetRow(0));
        key(&mut m, Key::Ok);
        assert_eq!(m.state.profile().user.pinned.get(&id).map(|k| k.engine.as_str()), Some("gfn"));
    }
    #[test]
    fn the_owners_achievements_are_asked_for_once_per_visit_and_kept() {
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
        let back = m.step(Input::Foreground(true));
        assert_eq!(sent(&back).iter().filter(|r| matches!(r, Request::Achievements { .. })).count(), 1, "asked again after the app comes back");
    }
    #[test]
    fn the_owners_achievements_in_a_game_steam_lists_only_as_a_record_are_asked_by_its_app_id() {
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
