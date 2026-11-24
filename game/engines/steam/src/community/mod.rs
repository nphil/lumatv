//! What Steam's community says about one game, for the game's page in the app: reviews, news, how many play now, how rare each
//! achievement is, the store's pictures and trailers, what players share on the hub, and its discussions and guides. The data is public,
//! so it is read the same signed in or out, and it is only ever sent to the app (see [`lumagame_core::community`]).
//!
//! Nothing here runs on its own: the app asks when the owner opens a page that shows it. The answer is kept in memory for an hour
//! and never written to disk. The pages are Steam's own and change without notice, so each section is fetched and read on its own and
//! is present or absent with a reason; a page that changed costs its own section and nothing else.
//!
//! The calls go one after another, each small and bounded: a time for the whole answer, a time for each call and a size for each body.
//! After two failed connections in a row the rest are not tried, and Steam asking for fewer requests (HTTP 429) stops every call for a
//! minute. A section that could not be reached is asked for again on the next ask and the ones that were read are kept.

mod api;
mod forum;
mod html;
mod hub;
mod text;

use crate::http::{self, COMMUNITY, Http, HttpError, Resp, STORE};
use crate::{Adapter, Clock};
use lumagame_core::community::{Absence, Community, MAX_GUIDES, MAX_HUB_ITEMS, MAX_NEWS, MAX_REVIEWS, MAX_URL, NewsItem, Section, Shot, Thread, Trailer};
use lumagame_core::ids::ProviderGameId;
use lumagame_core::ipc::{ErrorCode, Event};
use std::time::Duration;

/// The app shows one language, and the words Steam supplies itself (a score's name, a hub's labels) follow this.
const LANGUAGE: &str = "english";
/// How long an answer is kept, in memory only: a game page opened again within the hour asks Steam for nothing.
const TTL_MS: u64 = 3_600_000;
/// The most the whole answer may take, and the most one call may. A call is not started with less than `MIN_CALL_MS` left.
const BUDGET_MS: u64 = 12_000;
const CALL_MS: u64 = 8_000;
const MIN_CALL_MS: u64 = 1_000;
/// Failed connections in a row after which the rest of an answer is not tried: the network is down and waiting is no use.
const MAX_FAILURES: u8 = 2;
/// How long nothing is asked after Steam asked for fewer requests.
const COOLDOWN_MS: u64 = 60_000;
/// How many games and discussions are kept at once.
const GAMES_KEPT: usize = 12;
const THREADS_KEPT: usize = 8;
/// News is asked for a little longer than it is kept, because markup goes out of the text.
const NEWS_CHARS: usize = 1_500;

/// The hub's `appHubSubSection` for each kind of content.
const SCREENSHOTS: u8 = 2;
const VIDEOS: u8 = 3;
const ARTWORK: u8 = 4;
const GUIDES: u8 = 9;

/// What the engine remembers between asks, in memory only.
pub(crate) struct State {
    games: Memo<(u32, bool), Community>,
    threads: Memo<(u32, String, bool), Thread>,
    /// No call is made before this time (the clock's milliseconds): Steam asked for fewer requests.
    limited_until: u64,
}

impl Default for State {
    fn default() -> Self {
        Self { games: Memo::new(GAMES_KEPT), threads: Memo::new(THREADS_KEPT), limited_until: 0 }
    }
}

/// A few recent answers, each with when it was made.
struct Memo<K, V> {
    kept: Vec<(K, u64, V)>,
    cap: usize,
}

impl<K: PartialEq, V: Clone> Memo<K, V> {
    fn new(cap: usize) -> Self {
        Self { kept: Vec::new(), cap }
    }

    /// The answer for `key` and when it was made, unless it is [`TTL_MS`] old. Old answers are dropped here.
    fn get(&mut self, key: &K, now: u64) -> Option<(u64, V)> {
        self.kept.retain(|(_, at, _)| now.saturating_sub(*at) < TTL_MS);
        self.kept.iter().find(|(known, ..)| known == key).map(|(_, at, value)| (*at, value.clone()))
    }

    /// Keeps `value` for `key` as made at `at`. When full, the oldest answer goes.
    fn put(&mut self, key: K, at: u64, value: V) {
        self.kept.retain(|(known, ..)| *known != key);
        if self.kept.len() >= self.cap
            && let Some(oldest) = self.kept.iter().enumerate().min_by_key(|(_, (_, at, _))| *at).map(|(index, _)| index)
        {
            self.kept.remove(oldest);
        }
        self.kept.push((key, at, value));
    }
}

impl<H: Http, C: Clock> Adapter<H, C> {
    /// Answers [`Request::Community`](lumagame_core::ipc::Request::Community): every section for one game.
    pub(crate) fn community(&mut self, seq: u64, game: ProviderGameId, mature: bool) {
        let Some(app) = appid(&game) else { return self.error(seq, ErrorCode::BadRequest, "That is not a Steam game") };
        let now = self.clock.mono_ms();
        let key = (app, mature);
        let (at, mut community) = self.community.games.get(&key, now).unwrap_or_else(|| (now, unreachable()));
        // What is kept answers for itself while nothing in it can still be had, or while Steam has asked for quiet.
        if now < self.community.limited_until || !sections(&community).iter().any(|(_, absence)| *absence == Some(Absence::Unreachable)) {
            return self.answer(seq, Event::Community { game, community });
        }
        let mut fetch = Fetch::new(&self.http, &self.clock, app, mature);
        fetch.fill(app, &mut community);
        let limited = fetch.limited;
        if limited {
            self.community.limited_until = self.clock.mono_ms() + COOLDOWN_MS;
        }
        self.community.games.put(key, at, community.clone());
        self.note(&report(app, &community));
        self.answer(seq, Event::Community { game, community });
    }

    /// Answers [`Request::CommunityThread`](lumagame_core::ipc::Request::CommunityThread): one discussion's opening post and replies.
    pub(crate) fn community_thread(&mut self, seq: u64, game: ProviderGameId, topic: String, mature: bool) {
        let Some(app) = appid(&game) else { return self.error(seq, ErrorCode::BadRequest, "That is not a Steam game") };
        let Some((forum, number)) = forum::split_id(&topic) else { return self.error(seq, ErrorCode::BadRequest, "That is not a discussion topic") };
        let url = format!("{COMMUNITY}/app/{app}/discussions/{forum}/{number}/");
        let now = self.clock.mono_ms();
        let key = (app, topic.clone(), mature);
        let thread = if let Some((_, thread)) = self.community.threads.get(&key, now) {
            Section::Present { value: thread }
        } else if now < self.community.limited_until {
            Section::absent(Absence::Unreachable)
        } else {
            let mut fetch = Fetch::new(&self.http, &self.clock, app, mature);
            let thread = fetch.section(&url, http::PAGE_BODY, |resp| forum::thread(&String::from_utf8_lossy(&resp.body)));
            let limited = fetch.limited;
            if limited {
                self.community.limited_until = self.clock.mono_ms() + COOLDOWN_MS;
            }
            // Only a thread that was read is kept: one that could not be reached is worth asking for again at once.
            if let Section::Present { value } = &thread {
                self.community.threads.put(key, now, value.clone());
            }
            self.note(&format!("community thread {app}: {}", word(&thread)));
            thread
        };
        self.answer(seq, Event::CommunityThread { game, topic, thread });
    }
}

/// The game's Steam app id: the game's id here is the bare number.
fn appid(game: &ProviderGameId) -> Option<u32> {
    game.as_str().parse::<u32>().ok().filter(|appid| *appid != 0)
}

/// What a game that was never asked about holds: every section absent for want of an answer, so that all of them are due.
fn unreachable() -> Community {
    const GONE: Absence = Absence::Unreachable;
    Community {
        reviews: Section::absent(GONE),
        news: Section::absent(GONE),
        players: Section::absent(GONE),
        rarity: Section::absent(GONE),
        store_screenshots: Section::absent(GONE),
        trailers: Section::absent(GONE),
        hub_screenshots: Section::absent(GONE),
        hub_artwork: Section::absent(GONE),
        hub_videos: Section::absent(GONE),
        guides: Section::absent(GONE),
        discussions: Section::absent(GONE),
    }
}

/// Each section by name, in the order they are asked for, with why it is absent if it is.
fn sections(community: &Community) -> [(&'static str, Option<Absence>); 11] {
    fn state<T>(section: &Section<T>) -> Option<Absence> {
        match section {
            Section::Present { .. } => None,
            Section::Absent { reason } => Some(*reason),
        }
    }
    [
        ("reviews", state(&community.reviews)),
        ("players", state(&community.players)),
        ("news", state(&community.news)),
        ("rarity", state(&community.rarity)),
        ("store_screenshots", state(&community.store_screenshots)),
        ("trailers", state(&community.trailers)),
        ("hub_screenshots", state(&community.hub_screenshots)),
        ("hub_videos", state(&community.hub_videos)),
        ("hub_artwork", state(&community.hub_artwork)),
        ("guides", state(&community.guides)),
        ("discussions", state(&community.discussions)),
    ]
}

/// A section is due when it could not be reached last time. What Steam answered, even with nothing, is kept for the hour.
fn due<T>(section: &Section<T>) -> bool {
    matches!(section, Section::Absent { reason: Absence::Unreachable })
}

/// The log line for an answer: the app id, and the sections that are absent with their reason. Fixed words and numbers only.
fn report(app: u32, community: &Community) -> String {
    let absent: Vec<String> = sections(community).iter().filter_map(|(name, absence)| absence.map(|absence| format!("{name} {absence}"))).collect();
    if absent.is_empty() { format!("community {app}: all present") } else { format!("community {app}: absent: {}", absent.join(", ")) }
}

fn word<T>(section: &Section<T>) -> &'static str {
    match section {
        Section::Present { .. } => "present",
        Section::Absent { reason } => reason.as_str(),
    }
}

/// An address that is safe to hand on and to open: `https`, in one piece, and not long.
fn https(url: &str) -> Option<String> {
    let whole = url.len() > "https://".len() && url.len() <= MAX_URL && url.starts_with("https://");
    (whole && !url.chars().any(|c| c.is_control() || c.is_whitespace())).then(|| url.to_owned())
}

fn some(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// The items read, or `Unreadable` when there were items to read and none could be.
fn listed<T>(seen: usize, items: Vec<T>) -> Section<Vec<T>> {
    if items.is_empty() && seen > 0 { Section::absent(Absence::Unreadable) } else { Section::list(items) }
}

fn clamp32(number: u64) -> u32 {
    u32::try_from(number).unwrap_or(u32::MAX)
}

/// A count as the pages write it, with a separator for thousands; none for anything else.
fn count(text: &str) -> u32 {
    clamp32(text.chars().filter(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0))
}

/// The calls of one answer, and what they share: the time, the failures, and the age check.
struct Fetch<'a, H, C> {
    http: &'a H,
    clock: &'a C,
    /// When the whole answer must be done, in the clock's milliseconds.
    until: u64,
    /// Connections that failed one after another.
    failed: u8,
    /// Steam asked for fewer requests.
    limited: bool,
    /// What tells the community site the owner agreed to the age check. Sent to that site only, and only when the app says so.
    cookie: Option<String>,
}

impl<'a, H: Http, C: Clock> Fetch<'a, H, C> {
    fn new(http: &'a H, clock: &'a C, app: u32, mature: bool) -> Self {
        Self { http, clock, until: clock.mono_ms() + BUDGET_MS, failed: 0, limited: false, cookie: mature.then(|| format!("wants_mature_content_apps={app}")) }
    }

    /// Asks for every section that is due, one after another.
    fn fill(&mut self, app: u32, community: &mut Community) {
        if due(&community.reviews) {
            let url = http::api(&format!("/IUserReviewsService/GetAppReviews/v1/?appid={app}&num_per_page={MAX_REVIEWS}&filter=all&language={LANGUAGE}"));
            community.reviews = self.section(&url, http::SMALL_BODY, api::reviews);
        }
        if due(&community.players) {
            community.players =
                self.section(&http::api(&format!("/ISteamUserStats/GetNumberOfCurrentPlayers/v1/?appid={app}")), http::SMALL_BODY, api::players);
        }
        if due(&community.news) {
            community.news = self.news(app);
        }
        if due(&community.rarity) {
            let url = http::api(&format!("/ISteamUserStats/GetGlobalAchievementPercentagesForApp/v2/?gameid={app}&format=json"));
            community.rarity = self.section(&url, http::SMALL_BODY, api::rarity);
        }
        if due(&community.store_screenshots) || due(&community.trailers) {
            (community.store_screenshots, community.trailers) = self.store_media(app);
        }
        if due(&community.hub_screenshots) {
            community.hub_screenshots = self.hub(app, SCREENSHOTS, MAX_HUB_ITEMS, hub::items);
        }
        if due(&community.hub_videos) {
            community.hub_videos = self.hub(app, VIDEOS, MAX_HUB_ITEMS, hub::items);
        }
        if due(&community.hub_artwork) {
            community.hub_artwork = self.hub(app, ARTWORK, MAX_HUB_ITEMS, hub::items);
        }
        if due(&community.guides) {
            community.guides = self.hub(app, GUIDES, MAX_GUIDES, hub::guides);
        }
        if due(&community.discussions) {
            community.discussions =
                self.section(&format!("{COMMUNITY}/app/{app}/discussions/"), http::PAGE_BODY, |resp| forum::topics(&String::from_utf8_lossy(&resp.body)));
        }
    }

    /// The developer's own posts, and when there are none whatever feeds there are.
    fn news(&mut self, app: u32) -> Section<Vec<NewsItem>> {
        let all = http::api(&format!("/ISteamNews/GetNewsForApp/v2/?appid={app}&count={MAX_NEWS}&maxlength={NEWS_CHARS}&format=json"));
        match self.section(&format!("{all}&feeds=steam_community_announcements"), http::SMALL_BODY, api::news) {
            Section::Absent { reason: Absence::Empty } => self.section(&all, http::SMALL_BODY, api::news),
            found => found,
        }
    }

    /// The store page's screenshots and trailers, which are one answer.
    fn store_media(&mut self, app: u32) -> (Section<Vec<Shot>>, Section<Vec<Trailer>>) {
        match self.get(&format!("{STORE}/api/appdetails?appids={app}&l={LANGUAGE}&filters=screenshots,movies"), http::SMALL_BODY) {
            Ok(resp) => api::store_media(&resp),
            Err(reason) => (Section::absent(reason), Section::absent(reason)),
        }
    }

    /// One kind of what players share, or the guides, from the hub's content fragment.
    fn hub<T>(&mut self, app: u32, sub_section: u8, per_page: usize, read: fn(&str) -> Section<Vec<T>>) -> Section<Vec<T>> {
        let url =
            format!("{COMMUNITY}/app/{app}/homecontent/?appHubSubSection={sub_section}&render=1&numperpage={per_page}&p=1&browsefilter=trend&l={LANGUAGE}");
        self.section(&url, http::PAGE_BODY, |resp| read(&String::from_utf8_lossy(&resp.body)))
    }

    fn section<T>(&mut self, url: &str, limit: usize, read: impl FnOnce(&Resp) -> Section<T>) -> Section<T> {
        match self.get(url, limit) {
            Ok(resp) => read(&resp),
            Err(reason) => Section::absent(reason),
        }
    }

    /// One call, or why there is no answer to read.
    fn get(&mut self, url: &str, limit: usize) -> Result<Resp, Absence> {
        let left = self.until.saturating_sub(self.clock.mono_ms());
        if self.failed >= MAX_FAILURES || self.limited || left < MIN_CALL_MS {
            return Err(Absence::Unreachable);
        }
        let cookie = self.cookie.as_deref().filter(|_| url.strip_prefix(COMMUNITY).is_some_and(|path| path.starts_with('/')));
        let headers: Vec<(&str, &str)> = cookie.map(|value| ("Cookie", value)).into_iter().collect();
        match self.http.get_within(url, &headers, limit, Duration::from_millis(left.min(CALL_MS))) {
            Ok(resp) => match resp.status {
                200 => {
                    self.failed = 0;
                    Ok(resp)
                }
                // Steam's way of saying there is nothing here: no such game, no hub, no page, or sent elsewhere.
                204 | 300..=399 | 400 | 403 | 404 | 410 => {
                    self.failed = 0;
                    Err(Absence::Empty)
                }
                429 => {
                    self.limited = true;
                    Err(Absence::Unreachable)
                }
                500..=599 => {
                    self.failed = self.failed.saturating_add(1);
                    Err(Absence::Unreachable)
                }
                _ => {
                    self.failed = 0;
                    Err(Absence::Unreadable)
                }
            },
            Err(HttpError::Unreachable | HttpError::Timeout) => {
                self.failed = self.failed.saturating_add(1);
                Err(Absence::Unreachable)
            }
            Err(HttpError::TooLarge | HttpError::NotHttps) => Err(Absence::Unreadable),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{self, FakeClock, FakeHttp, Rig, down, ok, status};
    use lumagame_core::community::Thread;
    use lumagame_core::ipc::{FromEngine, Request};
    use std::cell::RefCell;
    use std::fs;
    use std::path::Path;
    use std::rc::Rc;

    const GAME: &str = "1091500";
    const TOPIC: &str = "0/565925967576627123";
    const REVIEWS: &str = include_str!("fixtures/reviews.json");
    const NEWS: &str = include_str!("fixtures/news.json");
    const PLAYERS: &str = include_str!("fixtures/players.json");
    const RARITY: &str = include_str!("fixtures/rarity.json");
    const APPDETAILS: &str = include_str!("fixtures/appdetails.json");
    const HUB_SCREENSHOTS: &str = include_str!("fixtures/hub_screenshots.html");
    const HUB_VIDEOS: &str = include_str!("fixtures/hub_videos.html");
    const HUB_ARTWORK: &str = include_str!("fixtures/hub_artwork.html");
    const HUB_GUIDES: &str = include_str!("fixtures/hub_guides.html");
    const DISCUSSIONS: &str = include_str!("fixtures/discussions.html");
    const THREAD: &str = include_str!("fixtures/thread.html");
    const AGE_GATE: &str = include_str!("fixtures/age_gate.html");

    /// The calls of one full answer, each by a word its address holds, and what each returns while Steam is well.
    const PAGES: [(&str, &str); 10] = [
        ("GetAppReviews", REVIEWS),
        ("GetNumberOfCurrentPlayers", PLAYERS),
        ("GetNewsForApp", NEWS),
        ("GetGlobalAchievementPercentagesForApp", RARITY),
        ("/api/appdetails", APPDETAILS),
        ("appHubSubSection=2", HUB_SCREENSHOTS),
        ("appHubSubSection=3", HUB_VIDEOS),
        ("appHubSubSection=4", HUB_ARTWORK),
        ("appHubSubSection=9", HUB_GUIDES),
        ("/discussions/", DISCUSSIONS),
    ];

    fn script(rig: &Rig) {
        for (needle, body) in PAGES {
            rig.http.on(needle, [ok(body)]);
        }
    }

    /// Every call of a full answer gets `reply`.
    fn script_all(rig: &Rig, reply: fn() -> Result<Resp, HttpError>) {
        for (needle, _) in PAGES {
            rig.http.on(needle, [reply()]);
        }
    }

    fn game() -> ProviderGameId {
        ProviderGameId::new(GAME).unwrap()
    }

    fn ask(rig: &mut Rig, mature: bool) -> Community {
        community_of(&rig.send(Request::Community { game: game(), mature }))
    }

    fn community_of(events: &[FromEngine]) -> Community {
        match events {
            [FromEngine { event: Event::Community { game: answered, community }, .. }] => {
                assert_eq!(answered.as_str(), GAME);
                community.clone()
            }
            other => panic!("expected one community event, got {other:?}"),
        }
    }

    fn open(rig: &mut Rig, topic: &str, mature: bool) -> Section<Thread> {
        let events = rig.send(Request::CommunityThread { game: game(), topic: topic.to_owned(), mature });
        match events.as_slice() {
            [FromEngine { event: Event::CommunityThread { game: answered, topic: asked, thread }, .. }] => {
                assert_eq!((answered.as_str(), asked.as_str()), (GAME, topic));
                thread.clone()
            }
            other => panic!("expected one thread event, got {other:?}"),
        }
    }

    fn error_of(events: &[FromEngine]) -> (ErrorCode, &str) {
        match events {
            [FromEngine { event: Event::Error { code, message }, .. }] => (*code, message),
            other => panic!("expected one error, got {other:?}"),
        }
    }

    fn absent_names(community: &Community) -> Vec<&'static str> {
        sections(community).iter().filter(|(_, absence)| absence.is_some()).map(|(name, _)| *name).collect()
    }

    fn files(dir: &Path) -> usize {
        fs::read_dir(dir).map_or(0, |entries| entries.flatten().map(|entry| if entry.path().is_dir() { files(&entry.path()) } else { 1 }).sum())
    }

    #[test]
    fn one_ask_gives_every_section_with_one_call_each_and_writes_nothing() {
        let mut rig = Rig::ready();
        script(&rig);
        let community = ask(&mut rig, false);
        assert_eq!(absent_names(&community), Vec::<&str>::new());
        assert_eq!(community.players, Section::Present { value: 47_861 });
        assert_eq!(community.reviews.value().map(|reviews| reviews.top.len()), Some(3));
        assert_eq!(community.hub_videos.value().map(Vec::len), Some(2));
        assert_eq!(community.discussions.value().map(Vec::len), Some(3));
        let sent = rig.http.sent();
        assert_eq!(sent.len(), 10);
        assert!(sent.iter().all(|call| !call.post && call.headers.is_empty() && call.url.starts_with("https://") && call.url.contains(GAME)), "{sent:?}");
        assert_eq!(rig.log.lines().last().map(String::as_str), Some("community 1091500: all present"));
        assert_eq!(files(rig.dir.path()), 0, "an answer lives in memory only");
    }

    #[test]
    fn a_game_steam_has_nothing_on_is_empty_in_every_section() {
        let mut rig = Rig::ready();
        rig.http.on("GetAppReviews", [fake::eresult(42, r#"{"response":{}}"#)]);
        rig.http.on("GetNumberOfCurrentPlayers", [status(404)]);
        rig.http.on("GetNewsForApp", [status(403)]);
        rig.http.on("GetGlobalAchievementPercentagesForApp", [status(403)]);
        rig.http.on("/api/appdetails", [ok(r#"{"999999999":{"success":false}}"#)]);
        for sub in ["2", "3", "4", "9"] {
            rig.http.on(&format!("appHubSubSection={sub}"), [ok("")]);
        }
        rig.http.on("/discussions/", [status(302)]);
        let community = ask(&mut rig, false);
        assert!(sections(&community).iter().all(|(_, absence)| *absence == Some(Absence::Empty)), "{:?}", sections(&community));
        assert_eq!(rig.http.count("GetNewsForApp"), 2, "the developer's feed, then every feed");
    }

    #[test]
    fn news_falls_back_to_every_feed_when_the_developer_has_none() {
        let mut rig = Rig::ready();
        script(&rig);
        rig.http.on("feeds=", [ok(r#"{"appnews":{"appid":1,"newsitems":[],"count":0}}"#)]);
        rig.http.on("GetNewsForApp", [ok(NEWS)]);
        let community = ask(&mut rig, false);
        assert_eq!(community.news.value().map(Vec::len), Some(3));
        let calls = rig.http.to("GetNewsForApp");
        assert_eq!(calls.len(), 2);
        assert!(calls[0].url.contains("feeds=steam_community_announcements") && !calls[1].url.contains("feeds="));
    }

    #[test]
    fn an_answer_is_kept_for_an_hour_and_then_asked_for_again() {
        let mut rig = Rig::ready();
        script(&rig);
        let first = ask(&mut rig, false);
        rig.clock.advance_s(3_599);
        assert_eq!(ask(&mut rig, false), first);
        assert_eq!(rig.http.sent().len(), 10, "the second ask came from memory");
        rig.clock.advance_s(2);
        ask(&mut rig, false);
        assert_eq!(rig.http.sent().len(), 20);
    }

    #[test]
    fn only_what_could_not_be_reached_is_asked_for_again() {
        let mut rig = Rig::ready();
        script(&rig);
        rig.http.on("GetGlobalAchievementPercentagesForApp", [down(), ok(RARITY)]);
        let first = ask(&mut rig, false);
        assert_eq!(absent_names(&first), ["rarity"]);
        rig.http.forget();
        let second = ask(&mut rig, false);
        assert_eq!(absent_names(&second), Vec::<&str>::new());
        assert_eq!(second.reviews, first.reviews, "what was read is kept");
        let sent = rig.http.sent();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].url.contains("GetGlobalAchievementPercentagesForApp"));
        rig.clock.advance_s(3_601);
        rig.http.forget();
        ask(&mut rig, false);
        assert_eq!(rig.http.sent().len(), 10, "after an hour from the first ask everything is asked for again");
    }

    #[test]
    fn a_dead_network_is_tried_twice_and_not_once_per_section() {
        let mut rig = Rig::ready();
        script_all(&rig, down);
        let community = ask(&mut rig, false);
        assert_eq!(rig.http.sent().len(), 2);
        assert!(sections(&community).iter().all(|(_, absence)| *absence == Some(Absence::Unreachable)));
        script(&rig);
        rig.http.forget();
        let back = ask(&mut rig, false);
        assert_eq!(absent_names(&back), Vec::<&str>::new(), "an answer of nothing but failures is not kept");
        assert_eq!(rig.http.sent().len(), 10);
    }

    #[test]
    fn steam_asking_for_fewer_requests_stops_every_call_for_a_minute() {
        let mut rig = Rig::ready();
        script(&rig);
        rig.http.on("GetAppReviews", [status(429)]);
        let first = ask(&mut rig, false);
        assert_eq!(rig.http.sent().len(), 1, "nothing more is asked in that answer");
        assert!(sections(&first).iter().all(|(_, absence)| *absence == Some(Absence::Unreachable)));
        rig.clock.advance_s(59);
        ask(&mut rig, false);
        assert_eq!(open(&mut rig, TOPIC, false), Section::absent(Absence::Unreachable));
        assert_eq!(rig.http.sent().len(), 1, "nothing for the minute, for a discussion either");
        rig.http.on("GetAppReviews", [ok(REVIEWS)]);
        rig.clock.advance_s(2);
        let later = ask(&mut rig, false);
        assert_eq!(rig.http.sent().len(), 11);
        assert_eq!(absent_names(&later), Vec::<&str>::new());
    }

    /// A network that takes `step_ms` of the clock for every call, and notes the time each call was given.
    struct Slow {
        inner: FakeHttp,
        clock: FakeClock,
        step_ms: u64,
        timeouts: Rc<RefCell<Vec<Duration>>>,
    }

    impl Http for Slow {
        fn get(&self, url: &str, limit: usize) -> Result<Resp, HttpError> {
            self.inner.get(url, limit)
        }

        fn get_within(&self, url: &str, headers: &[(&str, &str)], limit: usize, timeout: Duration) -> Result<Resp, HttpError> {
            self.timeouts.borrow_mut().push(timeout);
            self.clock.advance_ms(self.step_ms);
            self.inner.get_within(url, headers, limit, timeout)
        }

        fn post_form(&self, url: &str, headers: &[(&str, &str)], body: &str, limit: usize) -> Result<Resp, HttpError> {
            self.inner.post_form(url, headers, body, limit)
        }
    }

    #[test]
    fn a_slow_answer_stops_at_its_time_and_the_rest_waits_for_the_next_ask() {
        let rig = Rig::new();
        script(&rig);
        let timeouts = Rc::new(RefCell::new(Vec::new()));
        let slow = Slow { inner: rig.http.clone(), clock: rig.clock.clone(), step_ms: 5_000, timeouts: Rc::clone(&timeouts) };
        let mut adapter = Adapter::new(slow, rig.clock.clone(), |_: &str| {});
        adapter.handle(1, Request::Hello(rig.hello_request()));
        let community = community_of(&adapter.handle(2, Request::Community { game: game(), mature: false }));
        let ms = |ms| Duration::from_millis(ms);
        assert_eq!(*timeouts.borrow(), [ms(8_000), ms(7_000), ms(2_000)], "each call gets what is left of the answer's time, up to its own limit");
        assert_eq!(
            absent_names(&community),
            ["rarity", "store_screenshots", "trailers", "hub_screenshots", "hub_videos", "hub_artwork", "guides", "discussions"]
        );
        assert!(sections(&community)[3..].iter().all(|(_, absence)| *absence == Some(Absence::Unreachable)));
    }

    #[test]
    fn the_age_check_cookie_goes_to_the_community_site_only_and_only_when_asked() {
        let mut rig = Rig::ready();
        script(&rig);
        ask(&mut rig, true);
        let sent = rig.http.sent();
        assert_eq!(sent.len(), 10);
        for call in &sent {
            let expected: &[(String, String)] = if call.url.starts_with("https://steamcommunity.com/") {
                &[("Cookie".to_owned(), "wants_mature_content_apps=1091500".to_owned())]
            } else {
                &[]
            };
            assert_eq!(call.headers, expected, "{}", call.url);
        }
        assert_eq!(sent.iter().filter(|call| !call.headers.is_empty()).count(), 5, "the four hub pages and the discussions");
        rig.http.forget();
        ask(&mut rig, false);
        assert_eq!(rig.http.sent().len(), 10, "an answer for an owner who did not agree is kept apart");
        assert!(rig.http.sent().iter().all(|call| call.headers.is_empty()));
    }

    #[test]
    fn a_hub_for_adults_is_age_gated_without_the_cookie_and_the_rest_is_unaffected() {
        let mut rig = Rig::ready();
        script(&rig);
        rig.http.on("/discussions/", [ok(AGE_GATE)]);
        let community = ask(&mut rig, false);
        assert_eq!(absent_names(&community), ["discussions"]);
        assert_eq!(community.discussions, Section::absent(Absence::AgeGated));
        assert_eq!(rig.log.lines().last().map(String::as_str), Some("community 1091500: absent: discussions age-gated"));
    }

    #[test]
    fn a_page_that_changed_costs_its_own_section_and_nothing_else() {
        let mut rig = Rig::ready();
        script(&rig);
        rig.http.on("appHubSubSection=3", [ok("<html><body>Maintenance</body></html>")]);
        rig.http.on("appHubSubSection=9", [status(404)]);
        rig.http.on("GetNumberOfCurrentPlayers", [ok("<html>")]);
        rig.http.on("/discussions/", [status(503)]);
        let community = ask(&mut rig, false);
        assert_eq!(community.hub_videos, Section::absent(Absence::Unreadable));
        assert_eq!(community.guides, Section::absent(Absence::Empty));
        assert_eq!(community.players, Section::absent(Absence::Unreadable));
        assert_eq!(community.discussions, Section::absent(Absence::Unreachable));
        assert_eq!(absent_names(&community), ["players", "hub_videos", "guides", "discussions"]);
        assert_eq!(
            rig.log.lines().last().map(String::as_str),
            Some("community 1091500: absent: players unreadable, hub_videos unreadable, guides empty, discussions unreachable")
        );
    }

    #[test]
    fn every_status_is_one_of_four_answers_and_only_connection_trouble_counts_as_a_failure() {
        let (http, clock) = (FakeHttp::default(), FakeClock::new());
        let cases: [(Result<Resp, HttpError>, Absence, u8); 14] = [
            (status(204), Absence::Empty, 0),
            (status(302), Absence::Empty, 0),
            (status(400), Absence::Empty, 0),
            (status(403), Absence::Empty, 0),
            (status(404), Absence::Empty, 0),
            (status(410), Absence::Empty, 0),
            (status(401), Absence::Unreadable, 0),
            (status(418), Absence::Unreadable, 0),
            (status(500), Absence::Unreachable, 1),
            (status(503), Absence::Unreachable, 1),
            (down(), Absence::Unreachable, 1),
            (Err(HttpError::Timeout), Absence::Unreachable, 1),
            (Err(HttpError::TooLarge), Absence::Unreadable, 0),
            (Err(HttpError::NotHttps), Absence::Unreadable, 0),
        ];
        for (reply, absence, failed) in cases {
            http.on("x", [reply.clone()]);
            let mut fetch = Fetch::new(&http, &clock, 1, false);
            assert_eq!(fetch.get("https://steamcommunity.com/x", 1_024).map(|_| ()), Err(absence), "{reply:?}");
            assert_eq!((fetch.failed, fetch.limited), (failed, false), "{reply:?}");
        }
        http.on("x", [status(429)]);
        let mut fetch = Fetch::new(&http, &clock, 1, false);
        assert_eq!(fetch.get("https://steamcommunity.com/x", 1_024).map(|_| ()), Err(Absence::Unreachable));
        assert!(fetch.limited, "429 stops the rest");
        http.on("x", [ok("fine")]);
        let mut fetch = Fetch::new(&http, &clock, 1, false);
        assert_eq!(fetch.get("https://steamcommunity.com/x", 1_024).unwrap().body, b"fine");
        assert_eq!(fetch.get("https://steamcommunity.com/x", 2), Err(Absence::Unreadable), "a body over the limit is not read");
    }

    #[test]
    fn a_discussion_is_read_from_its_own_page_and_kept_for_ten_minutes() {
        let mut rig = Rig::ready();
        rig.http.on("/discussions/0/565925967576627123/", [ok(THREAD)]);
        let thread = open(&mut rig, TOPIC, false);
        assert_eq!(
            thread.value().map(|thread| (thread.title.as_str(), thread.posts.len(), thread.replies)),
            Some(("Can Anyone Explain Ricochets to Me?", 3, 13))
        );
        let sent = rig.http.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].url.as_str(), sent[0].headers.is_empty()), ("https://steamcommunity.com/app/1091500/discussions/0/565925967576627123/", true));
        assert_eq!(open(&mut rig, TOPIC, false), thread);
        assert_eq!(rig.http.sent().len(), 1, "from memory");
        rig.clock.advance_s(3_601);
        open(&mut rig, TOPIC, false);
        assert_eq!(rig.http.sent().len(), 2);
        assert_eq!(rig.log.lines().last().map(String::as_str), Some("community thread 1091500: present"));
    }

    #[test]
    fn a_discussion_that_was_not_read_is_asked_for_again_and_one_for_adults_sends_the_cookie() {
        let mut rig = Rig::ready();
        rig.http.on("/discussions/", [down(), ok(AGE_GATE), ok(THREAD)]);
        assert_eq!(open(&mut rig, TOPIC, false), Section::absent(Absence::Unreachable));
        assert_eq!(open(&mut rig, TOPIC, false), Section::absent(Absence::AgeGated));
        assert!(open(&mut rig, TOPIC, true).value().is_some());
        let sent = rig.http.sent();
        assert_eq!(sent.len(), 3);
        assert_eq!(sent.iter().map(|call| call.headers.len()).collect::<Vec<_>>(), [0, 0, 1]);
        assert_eq!(sent[2].headers[0], ("Cookie".to_owned(), "wants_mature_content_apps=1091500".to_owned()));
    }

    #[test]
    fn a_discussion_that_is_gone_or_changed_says_which() {
        let mut rig = Rig::ready();
        rig.http.on("/discussions/", [status(404)]);
        assert_eq!(open(&mut rig, TOPIC, false), Section::absent(Absence::Empty));
        rig.http.on("/discussions/", [ok("<html><body>Oops</body></html>")]);
        assert_eq!(open(&mut rig, TOPIC, false), Section::absent(Absence::Unreadable));
    }

    #[test]
    fn a_game_or_a_topic_that_is_not_one_is_refused_before_anything_is_sent() {
        let mut rig = Rig::ready();
        for id in ["abc", "0", "4294967296", "steam:1"] {
            let events = rig.send(Request::Community { game: ProviderGameId::new(id).unwrap(), mature: false });
            assert_eq!(error_of(&events), (ErrorCode::BadRequest, "That is not a Steam game"), "{id}");
            let events = rig.send(Request::CommunityThread { game: ProviderGameId::new(id).unwrap(), topic: TOPIC.to_owned(), mature: false });
            assert_eq!(error_of(&events), (ErrorCode::BadRequest, "That is not a Steam game"), "{id}");
        }
        for topic in ["", "0", "0/", "../../x", "0/5/6", "0/5?x=1", "a/5", "0/ 5"] {
            let events = rig.send(Request::CommunityThread { game: game(), topic: topic.to_owned(), mature: false });
            assert_eq!(error_of(&events), (ErrorCode::BadRequest, "That is not a discussion topic"), "{topic:?}");
        }
        assert!(rig.http.sent().is_empty());
    }

    #[test]
    fn the_log_holds_fixed_words_and_numbers_and_never_an_address_or_the_cookie() {
        let mut rig = Rig::ready();
        script(&rig);
        rig.http.on("appHubSubSection=3", [ok("<html>")]);
        rig.http.on("/discussions/0/", [ok(THREAD)]);
        ask(&mut rig, true);
        open(&mut rig, TOPIC, true);
        let lines = rig.log.lines();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines.iter().all(|line| !["http", "steam", "Cookie", "wants_mature"].iter().any(|word| line.contains(word))), "{lines:?}");
    }

    #[test]
    fn the_memo_drops_the_oldest_when_full_and_anything_as_old_as_the_limit() {
        let mut memo = Memo::new(2);
        memo.put("a", 10, 1);
        memo.put("b", 20, 2);
        memo.put("c", 30, 3);
        assert_eq!(memo.get(&"a", 40), None, "the oldest made room");
        assert_eq!(memo.get(&"b", 40), Some((20, 2)));
        memo.put("b", 20, 22);
        assert_eq!(memo.get(&"b", 40), Some((20, 22)), "a new value replaces the old one");
        assert_eq!(memo.get(&"b", 20 + TTL_MS - 1), Some((20, 22)));
        assert_eq!(memo.get(&"b", 20 + TTL_MS), None);
        assert_eq!(memo.get(&"c", 20 + TTL_MS), Some((30, 3)));
    }

    #[test]
    fn helpers_keep_addresses_whole_counts_plain_and_numbers_in_range() {
        assert_eq!(https("https://a.invalid/x?y=1").as_deref(), Some("https://a.invalid/x?y=1"));
        for url in ["", "https://", "http://a.invalid/", "ftp://a.invalid/", "https://a b/", "https://a/\u{7}", "https://a/\n"] {
            assert_eq!(https(url), None, "{url:?}");
        }
        assert_eq!(https(&format!("https://a/{}", "x".repeat(MAX_URL))), None);
        assert_eq!((count("1,234"), count(" 56 "), count(""), count("none"), count("99999999999999999999999")), (1_234, 56, 0, 0, 0));
        assert_eq!((clamp32(7), clamp32(u64::MAX)), (7, u32::MAX));
        assert!(
            listed::<u8>(0, Vec::new()).value().is_none()
                && listed::<u8>(2, Vec::new()) == Section::absent(Absence::Unreadable)
                && listed(2, vec![1_u8]).value().is_some()
        );
    }
}
