//! What a game's page knows below its actions, section by section, and whether the game can start: play and progress from this TV and
//! the owner's account, what Steam's community says (asked for once while a game has the owner's attention), the game's own details,
//! and the one line that says an integration needs the owner. The Home stage, the quick menu and the game page read it.

use super::game::REST_MS;
use super::*;
use lumagame_core::community::{self, Absence, Community, Guide, HubItem, NewsItem, Reviews, Shot, Thread, Topic, Trailer};
use lumagame_core::ids::{ExternalId, ProviderGameId};
use lumagame_core::launch::Candidate;
use std::borrow::Cow;
use std::cmp::Reverse;

/// How long an answer from Steam is kept before the game is asked about again.
const REFRESH_MS: u64 = 10 * 60_000;
/// How long a request may stay unanswered before it counts as lost and may be sent again.
const LOST_MS: u64 = 30_000;
/// How many games' answers, and how many discussion threads, are kept; the longest held go first.
const KEEP_APPS: usize = 48;
const KEEP_THREADS: usize = 12;
/// How long a game counts as recently added.
const RECENT_S: u64 = 14 * 86_400;

/// A section of the game's page, which `Model::open_game` can open the page at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Activity,
    Community,
    Media,
    About,
}

/// One thing a section knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fact<T> {
    Present(T),
    /// Asked for, or about to be; nothing has come back yet.
    Loading,
    Absent(Reason),
    /// Needs the owner's account: signed out, or (`expired`) signed in before and no longer.
    SignIn {
        expired: bool,
    },
}

/// Why a fact is absent: what the engine says about a part of Steam's answer, and the two reasons that never reach Steam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Empty,
    Unreachable,
    Unreadable,
    AgeGated,
    Other,
    /// The game pages are switched off, or no enabled integration offers the data.
    Off,
    /// The game has no Steam id, so Steam has nothing to say about it.
    NoSteam,
}

impl From<Absence> for Reason {
    fn from(absence: Absence) -> Self {
        match absence {
            Absence::Empty => Self::Empty,
            Absence::Unreachable => Self::Unreachable,
            Absence::Unreadable => Self::Unreadable,
            Absence::AgeGated => Self::AgeGated,
            Absence::Other => Self::Other,
        }
    }
}

/// The one set of words every screen uses for an absent fact.
pub fn why(reason: Reason) -> &'static str {
    match reason {
        Reason::Empty => "Nothing here yet",
        Reason::Unreachable => "Steam can't be reached",
        Reason::Unreadable => "Steam's page could not be read",
        Reason::AgeGated => "Age-restricted on Steam",
        Reason::Other => "Not available",
        Reason::Off => "Off in Settings",
        Reason::NoSteam => "Not on Steam",
    }
}

/// Whether the game can be started now. An asleep PC counts as ready: Play wakes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    Ready,
    Cannot(Why),
}

/// What stops a game starting, the most fixable first: when every way to play is blocked the game carries the first of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Why {
    SignIn,
    Pair,
    Offline,
    Install,
}

impl Why {
    pub fn word(self) -> &'static str {
        match self {
            Self::SignIn => "Sign in",
            Self::Pair => "Pair",
            Self::Offline => "Offline",
            Self::Install => "Install",
        }
    }
}

impl Start {
    pub fn word(self) -> &'static str {
        match self {
            Self::Ready => "Ready",
            Self::Cannot(why) => why.word(),
        }
    }
}

/// Everything the page and the Home stage know about one game. Lists borrow from the model, so reading costs little.
#[derive(Clone, Debug, PartialEq)]
pub struct Facts<'a> {
    pub activity: ActivityFacts<'a>,
    pub community: CommunityFacts<'a>,
    pub media: MediaFacts<'a>,
    pub about: AboutFacts<'a>,
    pub start: Start,
    /// When the game first showed up in the library, in Unix seconds, while that is within the last 14 days: "Recently added".
    pub added: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ActivityFacts<'a> {
    pub play_time: Fact<PlayTime>,
    pub last_session: Fact<LastSession>,
    pub achievements: Fact<Achievements<'a>>,
    /// How many play it now.
    pub players: Fact<u32>,
    pub news: Fact<&'a [NewsItem]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommunityFacts<'a> {
    pub reviews: Fact<&'a Reviews>,
    pub discussions: Fact<&'a [Topic]>,
    pub guides: Fact<&'a [Guide]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaFacts<'a> {
    pub store_screenshots: Fact<&'a [Shot]>,
    pub trailers: Fact<&'a [Trailer]>,
    /// What players share in the game's community hub.
    pub screenshots: Fact<&'a [HubItem]>,
    pub artwork: Fact<&'a [HubItem]>,
    pub videos: Fact<&'a [HubItem]>,
}

/// What the game says about itself. None of it waits for an answer: it is there when the details are, and absent when they are not.
#[derive(Clone, Debug, PartialEq)]
pub struct AboutFacts<'a> {
    pub description: Fact<&'a str>,
    pub developer: Fact<&'a str>,
    pub release: Fact<Cow<'a, str>>,
    pub controller: Fact<&'a str>,
    pub genres: Fact<Vec<String>>,
}

/// How long the owner has played: the longest count in any one place, never a sum (see `activity`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayTime {
    pub seconds: u64,
    pub place: Place,
    /// The last two weeks, where a service says so.
    pub recent: Option<u64>,
}

/// The latest time the game ran. A session on this TV says when it began, how long it ran and the service it streamed from; a
/// service's own report may give a day and no more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastSession {
    pub at: u64,
    pub day_only: bool,
    pub place: Place,
    pub seconds: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Achievements<'a> {
    pub unlocked: usize,
    /// How many the game has, when the store says.
    pub total: Option<u32>,
    /// The owner's latest among the ones the store page names, newest first.
    pub latest: Vec<Achievement<'a>>,
    /// The ones the store page names that the owner has not unlocked, in the store's order.
    pub next: Vec<Achievement<'a>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Achievement<'a> {
    pub name: &'a str,
    pub icon: Option<&'a str>,
    /// When it was unlocked, in Unix seconds, where the account says.
    pub at: Option<u64>,
    /// The share of players that has it, in tenths of a percent (`885` is 88.5 %).
    pub rarity: Option<u16>,
}

/// The line the top bar and the quick menu show when an integration needs the owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attention {
    pub words: String,
}

/// Names a discussion topic in `Model::open_thread`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopicId(String);

impl TopicId {
    pub fn of(topic: &Topic) -> Self {
        Self(topic.id.clone())
    }
}

/// Steam's answers for the session: a game's by its app id, and a discussion thread's by its game and topic.
#[derive(Clone, Debug)]
pub(super) struct Cache {
    apps: Shelf<u32, Community>,
    threads: Shelf<(u32, TopicId), community::Section<Thread>>,
}

impl Default for Cache {
    fn default() -> Self {
        Self { apps: Shelf { keep: KEEP_APPS, items: Vec::new() }, threads: Shelf { keep: KEEP_THREADS, items: Vec::new() } }
    }
}

/// Answers by key, at most `keep` of them: the longest held makes room, never one whose request is still out.
#[derive(Clone, Debug)]
struct Shelf<K, T> {
    keep: usize,
    items: Vec<(K, Held<T>)>,
}

impl<K, T> Shelf<K, T> {
    fn find(&self, is: impl Fn(&K) -> bool) -> Option<&Held<T>> {
        self.items.iter().find(|(key, _)| is(key)).map(|(_, held)| held)
    }

    fn at(&mut self, is: impl Fn(&K) -> bool, make: impl FnOnce() -> K) -> &mut Held<T> {
        let at = match self.items.iter().position(|(key, _)| is(key)) {
            Some(at) => at,
            None => {
                if self.items.len() >= self.keep {
                    let oldest = self.items.iter().enumerate().filter(|(_, (_, held))| held.asked.is_none()).min_by_key(|(_, (_, held))| held.answer.as_ref().map_or(0, |a| a.at_ms));
                    if let Some((oldest, _)) = oldest {
                        self.items.swap_remove(oldest);
                    }
                }
                self.items.push((make(), Held::default()));
                self.items.len() - 1
            }
        };
        &mut self.items[at].1
    }
}

#[derive(Clone, Debug)]
struct Held<T> {
    answer: Option<Answer<T>>,
    /// The request on its way: when it went out (the app's clock) and whether it asked for pages kept behind an age check.
    asked: Option<(u64, bool)>,
}

impl<T> Default for Held<T> {
    fn default() -> Self {
        Self { answer: None, asked: None }
    }
}

#[derive(Clone, Debug)]
struct Answer<T> {
    value: T,
    at_ms: u64,
    /// Whether it was asked for with the age-checked pages: an answer made without them must not outlive the switch being turned off.
    mature: bool,
}

impl<T> Held<T> {
    /// The answer, if it was made under the same age-check setting as now.
    fn usable(&self, mature: bool) -> Option<&T> {
        self.answer.as_ref().filter(|a| a.mature == mature).map(|a| &a.value)
    }

    /// Whether to ask: no answer made under this setting is fresh, and no request is out that may still come back.
    fn due(&self, mature: bool, now_ms: u64) -> bool {
        let waiting = self.asked.is_some_and(|(at, _)| now_ms.saturating_sub(at) < LOST_MS);
        let fresh = self.answer.as_ref().is_some_and(|a| a.mature == mature && now_ms.saturating_sub(a.at_ms) < REFRESH_MS);
        !waiting && !fresh
    }

    /// An answer takes the setting its request had; one nobody asked for, or a second to a request sent twice, keeps the last one's.
    fn answered(&mut self, value: T, now_ms: u64) {
        let mature = self.asked.take().map(|(_, mature)| mature).or(self.answer.as_ref().map(|a| a.mature)).unwrap_or(false);
        self.answer = Some(Answer { value, at_ms: now_ms, mature });
    }
}

/// Why the community has nothing to read yet.
#[derive(Clone, Copy)]
enum Gap {
    Loading,
    Absent(Reason),
}

impl Gap {
    fn fact<T>(self) -> Fact<T> {
        match self {
            Self::Loading => Fact::Loading,
            Self::Absent(reason) => Fact::Absent(reason),
        }
    }
}

/// The words of a text, or nothing when it is empty.
fn words(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.is_empty())
}

/// A part of Steam's answer as a fact.
fn pick<'a, T: 'a, U>(community: Result<&'a Community, Gap>, part: impl FnOnce(&'a Community) -> &'a community::Section<T>, show: impl FnOnce(&'a T) -> U) -> Fact<U> {
    match community.map(part) {
        Ok(community::Section::Present { value }) => Fact::Present(show(value)),
        Ok(community::Section::Absent { reason }) => Fact::Absent((*reason).into()),
        Err(gap) => gap.fact(),
    }
}

/// A fact that is there when the text is not empty.
fn text<T>(value: Option<T>) -> Fact<T> {
    value.map_or(Fact::Absent(Reason::Empty), Fact::Present)
}

/// The Steam app id of a game: the first its sources give that is digits and not zero, which is what the Steam engine accepts.
pub(super) fn steam_app(game: &Game) -> Option<u32> {
    let ids = game.sources.iter().flat_map(|s| &s.entry.external).filter(|id| id.ns() == ExternalId::STEAM);
    ids.into_iter().find_map(|id| id.id().parse::<u32>().ok().filter(|app| *app != 0))
}

/// Who can read the owner's achievements in a game.
pub(super) enum Reader<'a> {
    /// No enabled engine can.
    Off,
    /// An engine can once the owner signs in (`expired`: again).
    SignIn { expired: bool },
    Linked { engine: &'a EngineId, target: &'a TargetId, ask: Ask<'a> },
}

/// The id an engine knows the game by: its own, or the Steam app id for an account engine whose library lists no games.
#[derive(Clone, Copy)]
pub(super) enum Ask<'a> {
    Source(&'a ProviderGameId),
    App(u32),
}

impl Model {
    /// What the page and the Home stage know about a game. Only what is already held: nothing here asks for anything.
    pub fn facts(&self, id: GameId) -> Facts<'_> {
        let game = self.game(id);
        let community = self.community_of(game);
        let rarity = community.ok().and_then(|c| c.rarity.value());
        Facts {
            activity: ActivityFacts {
                play_time: self.play_time(id),
                last_session: self.last_session(id),
                achievements: self.achievements(game, id, rarity),
                players: pick(community, |c| &c.players, |n| *n),
                news: pick(community, |c| &c.news, Vec::as_slice),
            },
            community: CommunityFacts {
                reviews: pick(community, |c| &c.reviews, |r| r),
                discussions: pick(community, |c| &c.discussions, Vec::as_slice),
                guides: pick(community, |c| &c.guides, Vec::as_slice),
            },
            media: MediaFacts {
                store_screenshots: pick(community, |c| &c.store_screenshots, Vec::as_slice),
                trailers: pick(community, |c| &c.trailers, Vec::as_slice),
                screenshots: pick(community, |c| &c.hub_screenshots, Vec::as_slice),
                artwork: pick(community, |c| &c.hub_artwork, Vec::as_slice),
                videos: pick(community, |c| &c.hub_videos, Vec::as_slice),
            },
            about: self.about(id),
            start: self.can_start(id),
            added: self.added(id),
        }
    }

    /// One discussion thread of a game, once `open_thread` has asked for it.
    pub fn thread(&self, id: GameId, topic: &TopicId) -> Fact<&Thread> {
        let app = match self.community_gate(self.game(id)) {
            Ok((app, _)) => app,
            Err(gap) => return gap.fact(),
        };
        let held = self.answers.threads.find(|k| k.0 == app && k.1 == *topic).and_then(|held| held.usable(self.prefs.mature_pages));
        match held {
            Some(community::Section::Present { value }) => Fact::Present(value),
            Some(community::Section::Absent { reason }) => Fact::Absent((*reason).into()),
            None => Fact::Loading,
        }
    }

    /// Asks Steam for a topic's first posts, the moment it is opened. A thread already held or on its way is not asked for again.
    pub fn open_thread(&mut self, id: GameId, topic: TopicId, out: &mut Vec<Command>) {
        let Ok((app, engine)) = self.community_gate(self.game(id)) else { return };
        let (mature, now) = (self.prefs.mature_pages, self.now_ms);
        if self.answers.threads.find(|k| k.0 == app && k.1 == topic).is_some_and(|held| !held.due(mature, now)) {
            return;
        }
        let engine = engine.clone();
        let Ok(game) = ProviderGameId::new(app.to_string()) else { return };
        self.answers.threads.at(|k| k.0 == app && k.1 == topic, || (app, topic.clone())).asked = Some((now, mature));
        Self::send(out, &engine, Request::CommunityThread { game, topic: topic.0, mature });
    }

    /// Whether the game can be started now, without the rest of its facts: what a tile needs.
    pub fn can_start(&self, id: GameId) -> Start {
        let Some(game) = self.game(id) else { return Start::Cannot(Why::Install) };
        let mut blocked: Option<Why> = None;
        for source in &game.sources {
            let engine = self.integration(&source.engine);
            let target = engine.and_then(|e| e.targets.iter().find(|t| t.id == source.target));
            let why = match Candidate::of(source, target).readiness {
                Readiness::Ready | Readiness::Asleep | Readiness::Unknown => return Start::Ready,
                Readiness::SignIn if engine.is_some_and(|e| e.manifest.kind == EngineKind::Host) => Why::Pair,
                Readiness::SignIn => Why::SignIn,
                Readiness::Offline => Why::Offline,
                Readiness::Install => Why::Install,
            };
            blocked = Some(blocked.map_or(why, |first| first.min(why)));
        }
        Start::Cannot(blocked.unwrap_or(Why::Install))
    }

    /// When the game first showed up in the library, in Unix seconds, while that is within the last 14 days.
    pub fn added(&self, id: GameId) -> Option<u64> {
        let at = *self.state.profile().user.first_seen.get(&id)?;
        (self.unix_s.saturating_sub(at) <= RECENT_S).then_some(at)
    }

    /// The one thing an enabled integration needs from the owner, in a sentence: a sign-in or pairing, a PC that is off, or a refresh
    /// that failed. The first in the integrations' order; none while everything works.
    pub fn attention(&self) -> Option<Attention> {
        self.integrations.iter().filter(|e| self.enabled(&e.manifest.id)).find_map(trouble).map(|words| Attention { words })
    }

    fn play_time(&self, id: GameId) -> Fact<PlayTime> {
        text(self.play_time_of(id).map(|(seconds, place)| PlayTime { seconds, place, recent: self.recent_play(id) }))
    }

    fn last_session(&self, id: GameId) -> Fact<LastSession> {
        let Some(last) = self.last_played(id) else { return Fact::Absent(Reason::Empty) };
        let own = match last.place {
            Place::ThisTv => self.state.profile().history.sessions().rev().find(|p| p.game == id && p.start + u64::from(p.seconds) == last.at),
            Place::Service(_) => None,
        };
        Fact::Present(match own {
            Some(p) => LastSession { at: p.start, day_only: false, place: Place::Service(p.source.engine.clone()), seconds: Some(p.seconds) },
            None => LastSession { at: last.at, day_only: last.day_only, place: last.place, seconds: None },
        })
    }

    /// The owner's own progress. Names and icons exist only for the achievements the store page names; the rest only count.
    fn achievements<'a>(&'a self, game: Option<&'a Game>, id: GameId, rarity: Option<&'a BTreeMap<String, u16>>) -> Fact<Achievements<'a>> {
        let reader = game.map_or(Reader::Off, |g| self.reader(g));
        let unlocked = match (&reader, self.owned.get(&id)) {
            (Reader::Off, _) => return Fact::Absent(if game.and_then(steam_app).is_some() { Reason::Off } else { Reason::NoSteam }),
            (Reader::SignIn { expired }, None) => return Fact::SignIn { expired: *expired },
            (Reader::Linked { .. }, None) => return Fact::Loading,
            (_, Some(unlocked)) if unlocked.is_empty() => return Fact::Absent(Reason::Empty),
            (_, Some(unlocked)) => unlocked,
        };
        let store = self.details.get(&id).map(|d| &d.hub.achievements);
        let total = store.map(|s| s.total).filter(|n| *n > 0).or_else(|| rarity.map(|r| r.len() as u32).filter(|n| *n > 0));
        let named = store.map_or(&[][..], |s| s.shown.as_slice());
        let show = |a: &'a lumagame_meta::hub::Achievement, at: Option<u64>| Achievement {
            name: &a.name,
            icon: Some(a.icon.as_str()).filter(|icon| !icon.is_empty()),
            at,
            rarity: a.api.as_ref().and_then(|api| rarity?.get(api)).copied().or_else(|| a.permille.map(|p| p.min(1000) as u16)),
        };
        let mut latest: Vec<_> = named
            .iter()
            .filter_map(|a| {
                let have = unlocked.iter().find(|u| Some(&u.name) == a.api.as_ref())?;
                Some(show(a, have.at))
            })
            .collect();
        latest.sort_by_key(|a| Reverse(a.at));
        let next = named.iter().filter(|a| a.api.as_ref().is_some_and(|api| !unlocked.iter().any(|u| u.name == *api))).map(|a| show(a, None)).collect();
        Fact::Present(Achievements { unlocked: unlocked.len(), total, latest, next })
    }

    fn about(&self, id: GameId) -> AboutFacts<'_> {
        let details = self.details.get(&id);
        let genres = self.game_genres(id);
        AboutFacts {
            description: text(details.and_then(|d| words(Some(&d.description)))),
            developer: text(details.and_then(|d| words(d.developer.as_deref()))),
            release: text(details.and_then(|d| words(d.hub.release.as_deref()).map(Cow::Borrowed).or_else(|| d.year.map(|y| Cow::Owned(y.to_string()))))),
            controller: text(details.and_then(|d| words(d.hub.controller.as_deref()).or_else(|| d.capabilities.iter().map(String::as_str).find(|c| c.ends_with("controller support"))))),
            genres: text((!genres.is_empty()).then_some(genres)),
        }
    }

    /// Who can read the owner's achievements in the game: an engine that lists it, or any account engine for a game with a Steam id,
    /// since Steam lists its library as records, which are not games.
    pub(super) fn reader<'a>(&'a self, game: &'a Game) -> Reader<'a> {
        let app = steam_app(game);
        let mut sign_in: Option<bool> = None;
        for e in &self.integrations {
            if !self.enabled(&e.manifest.id) || !e.descriptor.as_ref().is_some_and(|d| d.capabilities.has(Feature::Achievements)) {
                continue;
            }
            let own = game.sources.iter().find(|s| s.engine == e.manifest.id);
            let ask = match (own, app) {
                (Some(s), _) => Ask::Source(&s.entry.id),
                (None, Some(app)) => Ask::App(app),
                (None, None) => continue,
            };
            let mut expired = false;
            for t in e.targets.iter().filter(|t| own.is_none_or(|s| s.target == t.id)) {
                if t.link == ipc::Link::Linked {
                    return Reader::Linked { engine: &e.manifest.id, target: &t.id, ask };
                }
                expired |= t.link == ipc::Link::Expired;
            }
            sign_in = Some(sign_in.unwrap_or(false) | expired);
        }
        sign_in.map_or(Reader::Off, |expired| Reader::SignIn { expired })
    }

    /// The first enabled integration that answers for Steam's community, while the game pages are on.
    fn community_engine(&self) -> Option<&EngineId> {
        if !self.prefs.game_pages {
            return None;
        }
        let offers = |e: &&Integration| self.enabled(&e.manifest.id) && e.descriptor.as_ref().is_some_and(|d| d.capabilities.has(Feature::Community));
        self.integrations.iter().find(offers).map(|e| &e.manifest.id)
    }

    /// The Steam app and the engine to ask about it, or why there is nothing to ask.
    fn community_gate(&self, game: Option<&Game>) -> Result<(u32, &EngineId), Gap> {
        let app = game.and_then(steam_app).ok_or(Gap::Absent(Reason::NoSteam))?;
        let engine = self.community_engine().ok_or(Gap::Absent(Reason::Off))?;
        Ok((app, engine))
    }

    fn community_of(&self, game: Option<&Game>) -> Result<&Community, Gap> {
        let (app, _) = self.community_gate(game)?;
        self.answers.apps.find(|k| *k == app).and_then(|held| held.usable(self.prefs.mature_pages)).ok_or(Gap::Loading)
    }

    /// The game whose page data is wanted now: the open page's, or the one in focus once it has rested.
    fn attended(&self) -> Option<GameId> {
        match self.view.page {
            Page::Game => self.view.selected,
            Page::Home | Page::Library => {
                let id = self.view.featured?;
                let (rested, since) = self.resting?;
                (rested == id && self.now_ms.saturating_sub(since) >= REST_MS).then_some(id)
            }
            _ => None,
        }
    }

    /// Asks for what the attended game's page shows that is not held yet: Steam's community, and the owner's achievements.
    pub(super) fn facts_ahead(&mut self, out: &mut Vec<Command>) {
        let Some(id) = self.attended() else { return };
        self.request_community(id, out);
        self.request_owned(id, out);
    }

    fn request_community(&mut self, id: GameId, out: &mut Vec<Command>) {
        let (mature, now) = (self.prefs.mature_pages, self.now_ms);
        let Ok((app, engine)) = self.community_gate(self.game(id)) else { return };
        if self.answers.apps.find(|k| *k == app).is_some_and(|held| !held.due(mature, now)) {
            return;
        }
        let engine = engine.clone();
        let Ok(game) = ProviderGameId::new(app.to_string()) else { return };
        self.answers.apps.at(|k| *k == app, || app).asked = Some((now, mature));
        Self::send(out, &engine, Request::Community { game, mature });
    }

    pub(super) fn community_answer(&mut self, game: &ProviderGameId, community: Community) {
        let Ok(app) = game.as_str().parse::<u32>() else { return };
        let now = self.now_ms;
        self.answers.apps.at(|k| *k == app, || app).answered(community, now);
    }

    pub(super) fn thread_answer(&mut self, game: &ProviderGameId, topic: String, thread: community::Section<Thread>) {
        let Ok(app) = game.as_str().parse::<u32>() else { return };
        let (topic, now) = (TopicId(topic), self.now_ms);
        self.answers.threads.at(|k| k.0 == app && k.1 == topic, || (app, topic.clone())).answered(thread, now);
    }

    /// The games a finished listing of one target brings that no listing of it held before. A target's first listing brings none:
    /// everything in it is old news. `known` holds the games there were before, so a game another service already lists is not new.
    pub(super) fn arrivals(&self, engine: &EngineId, target: &TargetId, entries: &[ipc::LibraryEntry]) -> Arrivals {
        let mut before = self.listing.sources.iter().filter(|s| s.engine == *engine && s.target == *target).map(|s| &s.entry.id).peekable();
        if before.peek().is_none() {
            return Arrivals::default();
        }
        let before: BTreeSet<&ProviderGameId> = before.collect();
        let fresh: Vec<SourceKey> = entries.iter().filter(|e| !before.contains(&e.id)).map(|e| SourceKey { engine: engine.clone(), target: target.clone(), id: e.id.clone() }).collect();
        let known = if fresh.is_empty() { BTreeSet::new() } else { self.games.iter().map(|g| g.id).collect() };
        Arrivals { fresh, known }
    }

    /// Stamps the games `arrivals` named, now that the catalog has been rebuilt, and drops the stamps that are no longer recent.
    pub(super) fn note_arrivals(&mut self, arrivals: Arrivals) {
        // Until the clock is known a stamp of zero would read as added long ago, and win every merge.
        if arrivals.fresh.is_empty() || self.unix_s == 0 {
            return;
        }
        let now = self.unix_s;
        let new: Vec<GameId> = self.games.iter().filter(|g| !arrivals.known.contains(&g.id) && g.sources.iter().any(|s| arrivals.fresh.contains(&s.key()))).map(|g| g.id).collect();
        let seen = &mut self.state.profile_mut().user.first_seen;
        for id in new {
            seen.entry(id).or_insert(now);
        }
        seen.retain(|_, at| now.saturating_sub(*at) <= RECENT_S);
    }
}

/// The new sources of one finished listing, and the games there were before it.
#[derive(Default)]
pub(super) struct Arrivals {
    fresh: Vec<SourceKey>,
    known: BTreeSet<GameId>,
}

/// What an integration needs from the owner, in a sentence. An engine with nothing to sign in to or pair has nothing the owner can mend.
fn trouble(e: &Integration) -> Option<String> {
    use lumagame_core::ipc::{ErrorCode, Link, Reach};
    if e.descriptor.as_ref().is_some_and(|d| d.capabilities.auth.is_empty()) {
        return None;
    }
    let name = e.manifest.name.as_str();
    match e.error.as_ref().map(|(code, _)| *code) {
        Some(ErrorCode::SignIn) => return Some(format!("{name} needs sign-in")),
        Some(ErrorCode::Network) => return Some(format!("{name} can't be reached")),
        Some(ErrorCode::Internal | ErrorCode::Other) => return Some(format!("{name} could not be refreshed")),
        // A mismatch of contracts or a request the engine refuses is not the owner's to mend.
        Some(ErrorCode::Protocol | ErrorCode::Unsupported | ErrorCode::BadRequest) | None => {}
    }
    let host = e.manifest.kind == EngineKind::Host;
    if let Some(t) = e.targets.iter().find(|t| t.link == Link::Expired) {
        return Some(if host { format!("Pair {} again", t.name) } else { format!("{name} sign-in has expired") });
    }
    if !host {
        return None;
    }
    let linked = || e.targets.iter().filter(|t| t.link == Link::Linked);
    let mut pcs = linked();
    match (pcs.next(), pcs.next()) {
        (None, _) => e.targets.iter().find(|t| t.link == Link::Found).map(|t| format!("Pair {}", t.name)),
        (Some(pc), None) => (pc.reach == Reach::Offline).then(|| format!("{} is offline", pc.name)),
        (Some(_), Some(_)) => linked().all(|t| t.reach == Reach::Offline).then(|| "Your PCs are offline".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::tests::*;
    use lumagame_core::activity::Activity;
    use lumagame_core::engine::AuthMethod;
    use lumagame_core::ipc::{ErrorCode, Link, Reach, Unlocked};
    use lumagame_meta::hub::{self, Hub};

    const HADES: &str = "1145350";

    /// One way to play a game: its engine, and what its target and its entry say.
    type Way = (&'static str, Link, Reach, Readiness);

    fn game(id: &str, title: &str, external: &[&str]) -> ipc::LibraryEntry {
        entry(id, title, ipc::Role::Game, external, Activity::default())
    }

    /// An engine's target links and its whole library arrives, as in a refresh.
    fn list(m: &mut Model, engine: &str, target: &str, entries: Vec<ipc::LibraryEntry>) {
        engine_says(m, engine, ipc::Event::Targets { targets: vec![linked(target)] });
        engine_says(m, engine, ipc::Event::Library { target: TargetId::new(target).unwrap(), entries, next: None });
    }

    /// GeForce NOW lists Hades II (Steam app 1145350) and Quake (a GOG id only); Steam is on and offers `features`.
    fn library(features: &[Feature]) -> (Model, GameId, GameId) {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud), manifest("steam", "Steam", EngineKind::Account)]);
        describe(&mut m, "gfn", vec![AuthMethod::DeviceCode], &[]);
        describe(&mut m, "steam", vec![AuthMethod::Qr], features);
        list(&mut m, "gfn", "main", vec![game("1", "Hades II", &["steam:1145350"]), game("2", "Quake", &["gog:1207658"])]);
        let (hades, quake) = (id_of(&m, "Hades II"), id_of(&m, "Quake"));
        (m, hades, quake)
    }

    fn tick(m: &mut Model, now_ms: u64) -> Vec<Command> {
        m.step(Input::Tick { now_ms, unix_s: 0 })
    }

    /// The community requests among `commands`: the Steam app, and whether the age-checked pages were asked for.
    fn asked(commands: &[Command]) -> Vec<(&str, bool)> {
        sent(commands).into_iter().filter_map(|r| if let Request::Community { game, mature } = r { Some((game.as_str(), *mature)) } else { None }).collect()
    }

    fn answer(m: &mut Model, community: Community) {
        engine_says(m, "steam", ipc::Event::Community { game: ProviderGameId::new(HADES).unwrap(), community });
    }

    fn absent<T>(reason: Absence) -> community::Section<T> {
        community::Section::absent(reason)
    }

    fn words<T>(fact: Fact<T>) -> &'static str {
        match fact {
            Fact::Absent(reason) => why(reason),
            Fact::Present(_) => "present",
            Fact::Loading => "loading",
            Fact::SignIn { .. } => "sign in",
        }
    }

    /// Hades II on a cloud account and on a PC, each way as `ways` says it is.
    fn start_of(ways: &[Way]) -> Start {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud), manifest("pc", "Sunshine", EngineKind::Host)]);
        describe(&mut m, "gfn", vec![AuthMethod::DeviceCode], &[]);
        describe(&mut m, "pc", vec![AuthMethod::ShowPin], &[]);
        for (engine, _, _, readiness) in ways {
            let mut hades = game(engine, "Hades II", &["steam:1145350"]);
            hades.readiness = *readiness;
            list(&mut m, engine, "main", vec![hades]);
        }
        // A listing needs a linked target; the target changes under it afterwards.
        for (engine, link, reach, _) in ways {
            engine_says(&mut m, engine, ipc::Event::Targets { targets: vec![Target { link: *link, reach: *reach, ..linked("main") }] });
        }
        m.can_start(id_of(&m, "Hades II"))
    }

    #[test]
    fn an_absent_part_reads_in_one_set_of_words() {
        let (mut m, hades, quake) = library(&[Feature::Community]);
        let community = Community {
            news: absent(Absence::Empty),
            players: absent(Absence::Unreachable),
            guides: absent(Absence::Unreadable),
            discussions: absent(Absence::AgeGated),
            hub_videos: absent(Absence::Other),
            ..Community::default()
        };
        answer(&mut m, community);
        let f = m.facts(hades);
        assert_eq!(words(f.activity.news), "Nothing here yet");
        assert_eq!(words(f.activity.players), "Steam can't be reached");
        assert_eq!(words(f.community.guides), "Steam's page could not be read");
        assert_eq!(words(f.community.discussions), "Age-restricted on Steam");
        assert_eq!(words(f.media.videos), "Not available");
        assert_eq!(words(m.facts(quake).community.reviews), "Not on Steam");
        m.prefs.game_pages = false;
        assert_eq!(words(m.facts(hades).community.reviews), "Off in Settings", "switched off, a held answer is not shown");
    }

    #[test]
    fn only_a_steam_game_is_asked_about_and_only_when_the_pages_are_on_and_offered() {
        let (mut m, hades, quake) = library(&[Feature::Community]);
        m.open_game(quake, None);
        assert!(asked(&tick(&mut m, 1_000)).is_empty(), "a game with no Steam id has nothing to ask");
        m.prefs.game_pages = false;
        m.open_game(hades, None);
        assert!(asked(&tick(&mut m, 2_000)).is_empty(), "switched off");
        m.prefs.game_pages = true;
        m.state.engines.entry(EngineId::new("steam").unwrap()).or_default().enabled = false;
        assert!(asked(&tick(&mut m, 3_000)).is_empty(), "the integration is off");
        let (mut bare, hades, _) = library(&[]);
        bare.open_game(hades, None);
        assert!(asked(&tick(&mut bare, 1_000)).is_empty(), "the engine does not offer the community");
    }

    #[test]
    fn a_game_is_asked_once_while_it_rests_and_again_when_the_answer_is_old_or_lost() {
        let (mut m, hades, _) = library(&[Feature::Community]);
        m.view.featured = Some(hades);
        assert!(asked(&tick(&mut m, 10_000)).is_empty(), "focus has just arrived");
        assert!(asked(&tick(&mut m, 10_000 + REST_MS - 1)).is_empty());
        let t0 = 10_000 + REST_MS;
        assert_eq!(asked(&tick(&mut m, t0)), [(HADES, false)], "rested");
        assert!(asked(&tick(&mut m, t0 + LOST_MS - 1)).is_empty(), "one request in flight");
        let t1 = t0 + LOST_MS;
        assert_eq!(asked(&tick(&mut m, t1)).len(), 1, "an unanswered request is sent again");
        answer(&mut m, Community::default());
        assert!(asked(&tick(&mut m, t1 + REFRESH_MS - 1)).is_empty(), "an answer holds for ten minutes");
        assert_eq!(asked(&tick(&mut m, t1 + REFRESH_MS)).len(), 1);
        assert_eq!(words(m.facts(hades).activity.players), "Not available", "the old answer stays up while the new one is awaited");
    }

    #[test]
    fn the_age_check_follows_the_setting_and_an_answer_made_under_the_other_is_not_shown() {
        let (mut m, hades, _) = library(&[Feature::Community]);
        m.prefs.mature_pages = true;
        m.open_game(hades, None);
        assert_eq!(asked(&tick(&mut m, 1_000)), [(HADES, true)], "opening the page asks at once");
        answer(&mut m, Community { players: community::Section::Present { value: 8_412 }, ..Community::default() });
        assert_eq!(m.facts(hades).activity.players, Fact::Present(8_412));
        m.prefs.mature_pages = false;
        assert_eq!(m.facts(hades).activity.players, Fact::Loading);
        assert_eq!(asked(&tick(&mut m, 2_000)), [(HADES, false)]);
    }

    #[test]
    fn an_answer_fills_every_section_and_a_thread_follows_its_topic() {
        let (mut m, hades, _) = library(&[Feature::Community]);
        assert_eq!(m.facts(hades).community.reviews, Fact::Loading, "nothing has come yet");
        let topic = Topic { id: "0/42".into(), title: "Boons".into(), author: "a".into(), replies: 3, last_post: None, pinned: false, locked: false, preview: String::new() };
        let shot = Shot { thumb: "https://x/t.jpg".into(), full: "https://x/f.jpg".into() };
        let reviews = Reviews { label: Some("Overwhelmingly Positive".into()), percent: Some(96), positive: 77_956, negative: 3_248, top: Vec::new() };
        let community = Community {
            reviews: community::Section::Present { value: reviews.clone() },
            discussions: community::Section::list(vec![topic.clone()]),
            store_screenshots: community::Section::list(vec![shot.clone()]),
            ..Community::default()
        };
        answer(&mut m, community);
        let f = m.facts(hades);
        assert_eq!(f.community.reviews, Fact::Present(&reviews));
        assert_eq!(f.community.discussions, Fact::Present(&[topic.clone()][..]));
        assert_eq!(f.media.store_screenshots, Fact::Present(&[shot][..]));
        assert_eq!(f.media.trailers, Fact::Absent(Reason::Other), "a part the engine did not send");
        let id = TopicId::of(&topic);
        assert_eq!(m.thread(hades, &id), Fact::Loading);
        let mut out = Vec::new();
        m.open_thread(hades, id.clone(), &mut out);
        m.open_thread(hades, id.clone(), &mut out);
        assert!(matches!(sent(&out)[..], [Request::CommunityThread { topic, mature: false, .. }] if topic == "0/42"), "asked once");
        let thread = Thread { title: "Boons".into(), posts: vec![community::Post { author: "a".into(), at: None, text: "Hi".into() }], replies: 3 };
        engine_says(&mut m, "steam", ipc::Event::CommunityThread { game: ProviderGameId::new(HADES).unwrap(), topic: "0/42".into(), thread: community::Section::Present { value: thread.clone() } });
        assert_eq!(m.thread(hades, &id), Fact::Present(&thread));
    }

    #[test]
    fn the_answers_kept_are_bounded() {
        let (mut m, _, _) = library(&[Feature::Community]);
        for app in 1..=(KEEP_APPS as u32 + 5) {
            engine_says(&mut m, "steam", ipc::Event::Community { game: ProviderGameId::new(app.to_string()).unwrap(), community: Community::default() });
        }
        assert_eq!(m.answers.apps.items.len(), KEEP_APPS);
    }

    #[test]
    fn a_game_is_new_only_when_a_later_refresh_brings_it() {
        let (mut m, hades, _) = library(&[]);
        let day = 86_400;
        let gfn = |more: &[(&str, &str)]| {
            let mut all = vec![game("1", "Hades II", &["steam:1145350"]), game("2", "Quake", &["gog:1207658"])];
            all.extend(more.iter().map(|(id, title)| game(id, title, &[])));
            all
        };
        assert_eq!(m.added(hades), None, "present at the first import");
        // The clock is not known yet: a stamp of zero would read as added long ago.
        list(&mut m, "gfn", "main", gfn(&[("3", "Control")]));
        assert!(m.state.profile().user.first_seen.is_empty());
        m.step(Input::Tick { now_ms: 0, unix_s: 1_000 * day });
        list(&mut m, "gfn", "main", gfn(&[("3", "Control"), ("4", "Hexen")]));
        let (control, hexen) = (id_of(&m, "Control"), id_of(&m, "Hexen"));
        assert_eq!((m.added(control), m.added(hexen)), (None, Some(1_000 * day)), "a later refresh brings Hexen; Control was listed before");
        // A service's first listing is all old news, and a game another service already lists is not new.
        list(&mut m, "steam", "76561198000000000", vec![game("77", "Doom", &[])]);
        list(&mut m, "steam", "76561198000000000", vec![game("77", "Doom", &[]), game("78", "Quake", &["gog:1207658"]), game("79", "Heretic", &[])]);
        let (doom, quake, heretic) = (id_of(&m, "Doom"), id_of(&m, "Quake"), id_of(&m, "Heretic"));
        assert_eq!((m.added(doom), m.added(quake), m.added(heretic)), (None, None, Some(1_000 * day)));
        // "Recently" ends after 14 days.
        m.step(Input::Tick { now_ms: 0, unix_s: 1_014 * day });
        assert_eq!(m.added(hexen), Some(1_000 * day));
        m.step(Input::Tick { now_ms: 0, unix_s: 1_014 * day + 1 });
        assert_eq!((m.added(hexen), m.facts(hexen).added), (None, None));
    }

    #[test]
    fn what_an_integration_needs_reads_as_one_line() {
        let pc = |link, reach| Target { id: TargetId::new("pc").unwrap(), name: "Desk PC".into(), detail: None, avatar: None, link, reach };
        type Row<'a> = (&'a str, EngineKind, Vec<Target>, Option<ErrorCode>, Option<&'a str>);
        let rows: Vec<Row> = vec![
            ("GeForce NOW", EngineKind::Cloud, vec![linked("main")], None, None),
            ("GeForce NOW", EngineKind::Cloud, vec![], None, None),
            ("GeForce NOW", EngineKind::Cloud, vec![pc(Link::Expired, Reach::Online)], None, Some("GeForce NOW sign-in has expired")),
            ("Sunshine", EngineKind::Host, vec![pc(Link::Linked, Reach::Online)], None, None),
            ("Sunshine", EngineKind::Host, vec![pc(Link::Linked, Reach::Asleep)], None, None),
            ("Sunshine", EngineKind::Host, vec![pc(Link::Linked, Reach::Offline)], None, Some("Desk PC is offline")),
            ("Sunshine", EngineKind::Host, vec![pc(Link::Linked, Reach::Offline), linked("laptop")], None, None),
            ("Sunshine", EngineKind::Host, vec![pc(Link::Found, Reach::Online)], None, Some("Pair Desk PC")),
            ("Sunshine", EngineKind::Host, vec![pc(Link::Expired, Reach::Online)], None, Some("Pair Desk PC again")),
            ("Steam", EngineKind::Account, vec![linked("1")], Some(ErrorCode::SignIn), Some("Steam needs sign-in")),
            ("Steam", EngineKind::Account, vec![linked("1")], Some(ErrorCode::Network), Some("Steam can't be reached")),
            ("Steam", EngineKind::Account, vec![linked("1")], Some(ErrorCode::Internal), Some("Steam could not be refreshed")),
            ("Steam", EngineKind::Account, vec![linked("1")], Some(ErrorCode::Other), Some("Steam could not be refreshed")),
            ("Steam", EngineKind::Account, vec![linked("1")], Some(ErrorCode::BadRequest), None),
            ("Steam", EngineKind::Account, vec![linked("1")], Some(ErrorCode::Protocol), None),
        ];
        for (name, kind, targets, error, expected) in rows {
            let mut m = Model::new(State::default(), Listing::default(), vec![manifest("e", name, kind)]);
            describe(&mut m, "e", vec![AuthMethod::Qr], &[]);
            engine_says(&mut m, "e", ipc::Event::Targets { targets });
            if let Some(code) = error {
                engine_says(&mut m, "e", ipc::Event::Error { code, message: "x".into() });
            }
            assert_eq!(m.attention().map(|a| a.words).as_deref(), expected, "{name} {error:?}");
        }
    }

    #[test]
    fn attention_leaves_out_what_is_off_or_cannot_be_mended_and_names_the_first_that_can() {
        let manifests = vec![manifest("native", "This TV", EngineKind::Native), manifest("gfn", "GeForce NOW", EngineKind::Cloud), manifest("pc", "Sunshine", EngineKind::Host)];
        let mut m = Model::new(State::default(), Listing::default(), manifests);
        describe(&mut m, "native", vec![], &[]);
        describe(&mut m, "gfn", vec![AuthMethod::DeviceCode], &[]);
        describe(&mut m, "pc", vec![AuthMethod::ShowPin], &[]);
        engine_says(&mut m, "native", ipc::Event::Error { code: ErrorCode::Internal, message: "list".into() });
        assert_eq!(m.attention(), None, "an engine with no sign-in has nothing the owner can mend");
        let offline = Target { reach: Reach::Offline, ..linked("pc") };
        engine_says(&mut m, "pc", ipc::Event::Targets { targets: vec![offline] });
        assert_eq!(m.attention().map(|a| a.words).as_deref(), Some("pc is offline"));
        let expired = Target { link: Link::Expired, ..linked("main") };
        engine_says(&mut m, "gfn", ipc::Event::Targets { targets: vec![expired] });
        assert_eq!(m.attention().map(|a| a.words).as_deref(), Some("GeForce NOW sign-in has expired"), "the first integration in the list");
        m.state.engines.entry(EngineId::new("gfn").unwrap()).or_default().enabled = false;
        assert_eq!(m.attention().map(|a| a.words).as_deref(), Some("pc is offline"), "a disabled integration is left out");
        m.step(Input::EngineGone(EngineId::new("pc").unwrap()));
        assert_eq!(m.attention().map(|a| a.words).as_deref(), Some("Sunshine can't be reached"));
    }

    #[test]
    fn a_game_starts_unless_every_way_to_play_is_blocked() {
        use Readiness::{Install, Ready};
        let (online, asleep, offline) = (Reach::Online, Reach::Asleep, Reach::Offline);
        let table: Vec<(Vec<Way>, Start)> = vec![
            (vec![("gfn", Link::Linked, online, Ready)], Start::Ready),
            (vec![("pc", Link::Linked, asleep, Ready)], Start::Ready),
            (vec![("gfn", Link::Expired, online, Ready)], Start::Cannot(Why::SignIn)),
            (vec![("pc", Link::Found, online, Ready)], Start::Cannot(Why::Pair)),
            (vec![("pc", Link::Linked, offline, Ready)], Start::Cannot(Why::Offline)),
            (vec![("pc", Link::Linked, online, Install)], Start::Cannot(Why::Install)),
            (vec![("pc", Link::Linked, offline, Ready), ("gfn", Link::Expired, online, Ready)], Start::Cannot(Why::SignIn)),
            (vec![("gfn", Link::Expired, online, Ready), ("pc", Link::Linked, asleep, Ready)], Start::Ready),
        ];
        for (ways, expected) in table {
            assert_eq!(start_of(&ways), expected, "{ways:?}");
        }
    }

    #[test]
    fn progress_and_rarity_read_from_the_account_the_store_page_and_the_community() {
        let (mut m, hades, quake) = library(&[Feature::Achievements, Feature::Community]);
        assert_eq!(m.facts(hades).activity.achievements, Fact::SignIn { expired: false }, "Steam is on and nobody has signed in");
        assert_eq!(words(m.facts(quake).activity.achievements), "Not on Steam");
        let account = TargetId::new("76561198000000000").unwrap();
        engine_says(&mut m, "steam", ipc::Event::Targets { targets: vec![Target { link: Link::Expired, ..linked("76561198000000000") }] });
        assert_eq!(m.facts(hades).activity.achievements, Fact::SignIn { expired: true });
        engine_says(&mut m, "steam", ipc::Event::Targets { targets: vec![linked("76561198000000000")] });
        assert_eq!(m.facts(hades).activity.achievements, Fact::Loading, "linked: asked for, not yet answered");
        let unlocked = ["ACH_A", "ACH_B", "ACH_Z"].into_iter().zip([Some(100), Some(200), None]).map(|(name, at)| Unlocked { name: name.into(), at }).collect();
        engine_says(&mut m, "steam", ipc::Event::Achievements { target: account, game: ProviderGameId::new(HADES).unwrap(), unlocked });
        let named = |api: &str, name: &str, permille| hub::Achievement { api: Some(api.into()), name: name.into(), icon: format!("https://x/{api}.jpg"), permille };
        let shown = vec![named("ACH_A", "First", Some(999)), named("ACH_B", "Second", None), named("ACH_C", "Third", Some(90))];
        let hub = Hub { achievements: hub::Achievements { total: 50, shown, rarest: None }, ..Hub::default() };
        m.step(Input::Details { game: hades, details: Box::new(GameDetails::from_hub(hub)) });
        answer(&mut m, Community { rarity: community::Section::Present { value: BTreeMap::from([("ACH_A".to_owned(), 610), ("ACH_C".to_owned(), 80)]) }, ..Community::default() });
        let Fact::Present(a) = m.facts(hades).activity.achievements else { panic!("progress is known") };
        assert_eq!((a.unlocked, a.total), (3, Some(50)), "the unnamed one still counts");
        fn seen<'a>(l: &[Achievement<'a>]) -> Vec<(&'a str, Option<u64>, Option<u16>)> {
            l.iter().map(|a| (a.name, a.at, a.rarity)).collect()
        }
        assert_eq!(seen(&a.latest), [("Second", Some(200), None), ("First", Some(100), Some(610))], "newest first; the community's share wins over the store's");
        assert_eq!(seen(&a.next), [("Third", None, Some(80))]);
        assert_eq!(a.latest[0].icon, Some("https://x/ACH_B.jpg"));
    }

    #[test]
    fn play_time_and_the_last_session_say_where_and_for_how_long() {
        let (mut m, hades, quake) = library(&[Feature::Community]);
        assert_eq!(m.facts(hades).activity.play_time, Fact::Absent(Reason::Empty));
        assert_eq!(m.facts(hades).activity.last_session, Fact::Absent(Reason::Empty));
        let source = SourceKey { engine: EngineId::new("gfn").unwrap(), target: TargetId::new("main").unwrap(), id: ProviderGameId::new("1").unwrap() };
        m.state.profile_mut().history.record(Played { game: hades, source, start: 10_000, seconds: 4_320 });
        let f = m.facts(hades).activity;
        assert_eq!(f.play_time, Fact::Present(PlayTime { seconds: 4_320, place: Place::ThisTv, recent: None }));
        let gfn = Place::Service(EngineId::new("gfn").unwrap());
        assert_eq!(f.last_session, Fact::Present(LastSession { at: 10_000, day_only: false, place: gfn, seconds: Some(4_320) }));
        assert_eq!(m.facts(quake).activity.last_session, Fact::Absent(Reason::Empty));
    }

    #[test]
    fn about_reads_what_the_details_say() {
        let (mut m, hades, _) = library(&[]);
        assert_eq!(m.facts(hades).about.description, Fact::Absent(Reason::Empty), "never loading: the store may not answer");
        let hub = Hub { release: Some("30 Sep, 2025".into()), controller: Some("Full controller support".into()), ..Hub::default() };
        let details = GameDetails { description: "Escape the Underworld".into(), developer: Some("Supergiant Games".into()), genres: vec!["Roguelike".into()], year: Some(2025), hub, ..GameDetails::default() };
        m.step(Input::Details { game: hades, details: Box::new(details) });
        let about = m.facts(hades).about;
        assert_eq!(about.description, Fact::Present("Escape the Underworld"));
        assert_eq!(about.developer, Fact::Present("Supergiant Games"));
        assert_eq!(about.release, Fact::Present(Cow::Borrowed("30 Sep, 2025")));
        assert_eq!(about.controller, Fact::Present("Full controller support"));
        assert_eq!(about.genres, Fact::Present(vec!["Roguelike".to_owned()]));
    }
}
