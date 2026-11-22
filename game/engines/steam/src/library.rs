//! The account's games: which it owns, when each was last played, for how long, and what kind of game each is.
//!
//! Every game is a record, never a card: the app puts it on the game it already lists from another engine that shares its Steam id. The
//! first request of a round, the one without a cursor, is the only one that may go to Steam, and only when what Steam last said is
//! five minutes old or the owner asked for a refresh. What Steam last said is kept in `library.json` and served whenever Steam cannot
//! be reached, so the app can ask as often as it likes and a missing connection costs nothing.

use crate::auth::{Refusal, Trouble};
use crate::files;
use crate::http::{self, Http};
use crate::store::Store;
use crate::{Adapter, Clock};
use lumagame_core::activity::Activity;
use lumagame_core::ids::{ExternalId, ProviderGameId, TargetId};
use lumagame_core::ipc::{Art, ErrorCode, Event, LibraryEntry, Readiness, Role};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::io;
use std::path::Path;

pub const CACHE_FILE: &str = "library.json";
/// The most of the file that is read: a file left by something else may be any size.
const CACHE_MAX: u64 = 4 << 20;
/// Games in one page. A page of the widest entries there can be stays under three quarters of the protocol's line limit.
const PAGE: usize = 200;
const MAX_ENTRIES: usize = 1500;
/// How long what Steam last said is served without asking again.
const FRESH_S: u64 = 300;
const TITLE_CHARS: usize = 160;
const STALE_CURSOR: &str = "This library cursor is no longer current";

/// What `library.json` holds: the library as Steam last gave it.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    steamid: String,
    /// When Steam last answered, unix seconds. A failed refresh does not change it, so the next request tries again.
    fetched_s: u64,
    entries: Vec<LibraryEntry>,
}

impl Snapshot {
    /// The saved library of this account, or nothing when there is none, it is another account's, or it is not ours.
    pub fn load(dir: &Path, steamid: &str) -> Option<Self> {
        let snapshot: Self = serde_json::from_slice(&files::read(dir, CACHE_FILE, CACHE_MAX)?).ok()?;
        (snapshot.steamid == steamid && snapshot.entries.len() <= MAX_ENTRIES).then_some(snapshot)
    }

    fn save(&self, dir: &Path) -> io::Result<()> {
        files::write(dir, CACHE_FILE, &serde_json::to_vec(self).map_err(io::Error::other)?)
    }

    /// Due again. A time in the future is a clock that was wrong, and is not trusted.
    fn stale(&self, now: u64) -> bool {
        self.fetched_s > now || now - self.fetched_s >= FRESH_S
    }
}

/// One owned game as Steam reports it.
struct Game {
    appid: u32,
    name: Option<String>,
    minutes: u64,
    last_played: Option<u64>,
    /// Minutes played in the last two weeks.
    recent_minutes: u64,
}

/// A game's name for display: one line, at most 160 characters.
fn title(raw: &str) -> Option<String> {
    let name: String = raw.chars().filter(|c| !c.is_control()).take(TITLE_CHARS).collect();
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// The games in a `GetOwnedGames` answer, each once. `None` when the answer is not one. An account with no games has `{"response":{}}`.
fn parse_owned(body: &[u8]) -> Option<Vec<Game>> {
    #[derive(Deserialize)]
    struct Body {
        response: Fields,
    }
    #[derive(Deserialize)]
    struct Fields {
        #[serde(default)]
        games: Vec<Raw>,
    }
    // The numbers and the name are read loosely: one odd value in one game must not make the whole library unreadable.
    #[derive(Deserialize)]
    struct Raw {
        appid: u64,
        name: Option<Value>,
        playtime_forever: Option<Value>,
        rtime_last_played: Option<Value>,
        playtime_2weeks: Option<Value>,
    }
    let Body { response } = serde_json::from_slice(body).ok()?;
    let mut seen = HashSet::new();
    let games = response
        .games
        .into_iter()
        .filter_map(|raw| {
            let appid = u32::try_from(raw.appid).ok().filter(|appid| *appid != 0)?;
            seen.insert(appid).then(|| Game {
                appid,
                name: raw.name.as_ref().and_then(Value::as_str).and_then(title),
                minutes: raw.playtime_forever.as_ref().and_then(Value::as_u64).unwrap_or(0),
                // 0 is Steam's word for never.
                last_played: raw.rtime_last_played.as_ref().and_then(Value::as_u64).filter(|at| *at > 0),
                recent_minutes: raw.playtime_2weeks.as_ref().and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect();
    Some(games)
}

/// What was played last first, then what was played longest, then by id so that the order is the same every time.
fn order(games: &mut Vec<Game>) {
    games.sort_by_key(|game| (Reverse(game.last_played), Reverse(game.minutes), game.appid));
    games.truncate(MAX_ENTRIES);
}

fn entry(game: &Game, store: &Store) -> Option<LibraryEntry> {
    let appid = game.appid.to_string();
    let (genres, tags) = store.classify(game.appid);
    let mut activity = Activity {
        last_played: game.last_played,
        day_only: false,
        playtime_s: (game.minutes > 0).then(|| game.minutes.saturating_mul(60)),
        // Steam keeps Favorites in the client: the web API does not give them.
        favourite: false,
        recent_s: (game.recent_minutes > 0).then(|| game.recent_minutes.saturating_mul(60)),
        genres,
        tags,
    };
    activity.clamp();
    Some(LibraryEntry {
        id: ProviderGameId::new(appid.clone()).ok()?,
        title: game.name.clone().unwrap_or_else(|| format!("Steam app {appid}")),
        role: Role::Record,
        external: vec![ExternalId::new(ExternalId::STEAM, appid).ok()?],
        store: Some(ExternalId::STEAM.to_owned()),
        store_account: None,
        art: Art::default(),
        readiness: Readiness::default(),
        resumable: false,
        activity,
    })
}

/// The page that starts where `cursor` says, and the cursor of the page after it. `None` for a cursor this engine did not give out.
fn page(entries: &[LibraryEntry], cursor: Option<&str>) -> Option<(Vec<LibraryEntry>, Option<String>)> {
    let start = match cursor {
        None => 0,
        Some(text) => {
            // Exactly what was issued: the decimal offset of a page after the first.
            let start: usize = text.parse().ok().filter(|start: &usize| start.to_string() == text)?;
            (start > 0 && start.is_multiple_of(PAGE) && start < entries.len()).then_some(start)?
        }
    };
    let end = (start + PAGE).min(entries.len());
    Some((entries[start..end].to_vec(), (end < entries.len()).then(|| end.to_string())))
}

impl<H: Http, C: Clock> Adapter<H, C> {
    /// One page of the library. Without a cursor it is the start of a round: Steam is asked first when its last answer is old or
    /// the owner asked for a refresh, and the saved copy is served when that fails.
    pub(crate) fn library(&mut self, seq: u64, target: TargetId, cursor: Option<&str>) {
        let Some(account) = &self.account else {
            return self.error(seq, ErrorCode::BadRequest, "No Steam account is signed in");
        };
        if account.steamid != target.as_str() {
            return self.error(seq, ErrorCode::BadRequest, "That is not the signed-in Steam account");
        }
        let steamid = account.steamid.clone();
        if self.snapshot.as_ref().is_none_or(|snapshot| snapshot.steamid != steamid) {
            self.snapshot = Snapshot::load(&self.cache, &steamid);
        }
        let mut refusal = None;
        if cursor.is_none() {
            let forced = std::mem::take(&mut self.refresh_wanted);
            let now = self.clock.unix_s();
            if forced || self.snapshot.as_ref().is_none_or(|snapshot| snapshot.stale(now)) {
                refusal = self.refresh_library(&steamid).err();
            }
        }
        let paged = self.snapshot.as_ref().map(|snapshot| page(&snapshot.entries, cursor));
        match (paged, cursor) {
            (Some(Some((entries, next))), _) => self.answer(seq, Event::Library { target, entries, next }),
            (Some(None), _) | (None, Some(_)) => self.error(seq, ErrorCode::BadRequest, STALE_CURSOR),
            (None, None) => match refusal {
                Some(Refusal::Expired) => self.error(seq, ErrorCode::SignIn, "The Steam sign-in has expired and nothing is saved yet"),
                _ => self.error(seq, ErrorCode::Network, "Steam is not reachable and nothing is saved yet"),
            },
        }
    }

    /// Asks Steam for the library and the kinds of game in it, and keeps the answer. Trouble leaves what was saved as it was.
    fn refresh_library(&mut self, steamid: &str) -> Result<(), Refusal> {
        let outcome = self.fetch_library(steamid);
        match &outcome {
            Ok(()) => {}
            Err(Refusal::Expired) => self.note("library: not refreshed, Steam does not accept the sign-in"),
            Err(Refusal::Trouble(trouble)) => self.note(&format!("library: not refreshed, {trouble}")),
        }
        outcome
    }

    fn fetch_library(&mut self, steamid: &str) -> Result<(), Refusal> {
        let token = self.access_token()?;
        let query =
            http::form(&[("access_token", &token), ("steamid", steamid), ("include_appinfo", "1"), ("include_played_free_games", "1"), ("format", "json")]);
        let reply = self.http.get(&http::api(&format!("/IPlayerService/GetOwnedGames/v1/?{query}")), http::LIBRARY_BODY);
        let resp = reply.map_err(|error| Refusal::Trouble(Trouble::Network(error)))?;
        // Steam says a refused token with HTTP 401 or 403, or with 200 and result 15, whose body is empty and must not pass for an empty library.
        if matches!(resp.status, 401 | 403) || (resp.status == 200 && resp.eresult == Some(15)) {
            self.expire();
            return Err(Refusal::Expired);
        }
        if !resp.succeeded() {
            return Err(Refusal::Trouble(Trouble::of(&resp)));
        }
        let mut games = parse_owned(&resp.body).ok_or(Refusal::Trouble(Trouble::Unreadable))?;
        order(&mut games);
        let now = self.clock.unix_s();
        let wanted: Vec<u32> = games.iter().map(|game| game.appid).collect();
        // Taken out so that the store can be asked with the network borrowed alongside; nothing between here and the put-back can fail.
        let mut store = self.store.take().unwrap_or_else(|| Store::load(&self.cache));
        let done = store.refresh(&self.http, now, &wanted);
        if let Some(trouble) = done.stopped {
            self.note(&format!("library: kinds of game stopped after {} games, {trouble}", done.looked_up));
        }
        if done.changed
            && let Err(error) = store.save(&self.cache)
        {
            self.note(&format!("library: the kinds of game could not be saved ({:?})", error.kind()));
        }
        let entries: Vec<LibraryEntry> = games.iter().filter_map(|game| entry(game, &store)).collect();
        self.store = Some(store);
        let snapshot = Snapshot { steamid: steamid.to_owned(), fetched_s: now, entries };
        if let Err(error) = snapshot.save(&self.cache) {
            self.note(&format!("library: the library could not be saved ({:?})", error.kind()));
        }
        self.note(&format!("library: {} games from Steam", snapshot.entries.len()));
        self.snapshot = Some(snapshot);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{self, FakeHttp, Rig, STEAMID, UNIX0};
    use lumagame_core::ipc::{self, FromEngine, Link, Request};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const TAGS: &str = r#"{"response":{"version_hash":"7","tags":[{"tagid":9,"name":"Strategy"},{"tagid":19,"name":"Action"},{"tagid":122,"name":"Role-Playing"},{"tagid":1695,"name":"Open World"},{"tagid":29482,"name":"Souls-like"}]}}"#;
    const NOT_REACHABLE: &str = "Steam is not reachable and nothing is saved yet";

    fn target() -> TargetId {
        TargetId::new(STEAMID).unwrap()
    }

    /// A signed-in engine whose Steam gives `games`, a day-long access token, and the store's answer for apps 10 and 40.
    fn rig_with(games: &str) -> Rig {
        let rig = Rig::signed_in();
        let access = fake::jwt(STEAMID, Some(UNIX0 + 86_400));
        rig.http.on("GenerateAccessTokenForApp", [fake::ok(&format!(r#"{{"response":{{"access_token":"{access}"}}}}"#))]);
        rig.http.on("GetOwnedGames", [fake::ok(games)]);
        rig.http.on("IStoreService", [fake::ok(TAGS)]);
        let items = r#"{"response":{"store_items":[{"id":10,"success":1,"visible":true,"tagids":[29482,122,1695,19]},{"id":40,"success":1,"visible":true,"tagids":[9]}]}}"#;
        rig.http.on("IStoreBrowseService", [fake::ok(items)]);
        rig
    }

    fn library(rig: &mut Rig) -> Vec<FromEngine> {
        rig.send(Request::Library { target: target(), cursor: None })
    }

    fn cursor(rig: &mut Rig, cursor: &str) -> Vec<FromEngine> {
        rig.send(Request::Library { target: target(), cursor: Some(cursor.to_owned()) })
    }

    /// The entries and the next cursor of a reply that is one page.
    fn page_of(events: &[FromEngine]) -> (Vec<LibraryEntry>, Option<String>) {
        match events {
            [FromEngine { event: Event::Library { entries, next, .. }, .. }] => (entries.clone(), next.clone()),
            other => panic!("expected one page, got {other:?}"),
        }
    }

    fn ids(entries: &[LibraryEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.id.as_str()).collect()
    }

    fn message(events: &[FromEngine]) -> (ErrorCode, &str) {
        match events {
            [FromEngine { event: Event::Error { code, message }, .. }] => (*code, message),
            other => panic!("expected one error, got {other:?}"),
        }
    }

    fn many(count: u32) -> String {
        let games: Vec<(u32, &str, u64, u64)> = (1..=count).map(|id| (id, "Game", 1, 1_700_000_000 + u64::from(count - id))).collect();
        fake::owned(&games)
    }

    #[test]
    fn owned_games_become_records_ordered_by_what_was_played_and_named_by_the_store() {
        let games = fake::owned(&[(30, "Thirty", 5, 0), (20, "", 0, 0), (40, "Forty", 5, 1_700_000_100), (10, "Ten", 120, 1_700_000_100), (50, "Fifty", 5, 0)]);
        let mut rig = rig_with(&games);
        let (entries, next) = page_of(&library(&mut rig));
        assert_eq!(next, None);
        assert_eq!(ids(&entries), ["10", "40", "30", "50", "20"], "last played, then play time, then id; never played last");
        let ten = &entries[0];
        let expected = LibraryEntry {
            id: ProviderGameId::new("10").unwrap(),
            title: "Ten".to_owned(),
            role: Role::Record,
            external: vec![ExternalId::new("steam", "10").unwrap()],
            store: Some("steam".to_owned()),
            store_account: None,
            art: Art::default(),
            readiness: Readiness::default(),
            resumable: false,
            activity: Activity {
                last_played: Some(1_700_000_100),
                day_only: false,
                playtime_s: Some(7_200),
                favourite: false,
                recent_s: None,
                genres: vec!["RPG".to_owned(), "Action".to_owned()],
                tags: vec!["Souls-like".to_owned(), "Open World".to_owned()],
            },
        };
        assert_eq!(ten, &expected);
        assert_eq!(entries[1].activity.genres, ["Strategy"]);
        assert_eq!(entries[1].title, "Forty");
        let unnamed = &entries[4];
        assert_eq!((unnamed.title.as_str(), unnamed.activity.is_empty()), ("Steam app 20", true), "no name, never played, not known to the store");
        assert_eq!(entries[2].activity.playtime_s, Some(300));
        assert_eq!(entries[2].activity.last_played, None);

        let order: Vec<String> = rig.http.sent().iter().map(|sent| sent.url.split('?').next().unwrap().rsplit("com/").next().unwrap().to_owned()).collect();
        assert_eq!(
            order,
            [
                "IAuthenticationService/GenerateAccessTokenForApp/v1/",
                "IPlayerService/GetOwnedGames/v1/",
                "IStoreService/GetTagList/v1/",
                "IStoreBrowseService/GetItems/v1/"
            ]
        );
        let owned = &rig.http.to("GetOwnedGames")[0];
        assert!(!owned.post && owned.url.ends_with(&format!("&steamid={STEAMID}&include_appinfo=1&include_played_free_games=1&format=json")), "{}", owned.url);
        let mode = |name: &str| fs::metadata(rig.cache().join(name)).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode("library.json"), mode("store.json"), mode("")), (0o600, 0o600, 0o700));
    }

    #[test]
    fn steam_is_asked_at_most_every_five_minutes_and_a_restart_does_not_change_that() {
        let mut rig = rig_with(&fake::owned(&[(10, "Ten", 1, 1_700_000_000)]));
        let first = library(&mut rig);
        rig.clock.advance_s(299);
        assert_eq!(library(&mut rig), [FromEngine::answer(rig.seq, page_event(&first))]);
        assert_eq!(rig.http.count("GetOwnedGames"), 1, "299 seconds on: served from the saved copy");
        rig.clock.advance_s(1);
        library(&mut rig);
        assert_eq!(rig.http.count("GetOwnedGames"), 2, "at 300 seconds Steam is asked again");

        rig.restart();
        rig.hello();
        let served = library(&mut rig);
        assert_eq!(rig.http.count("GetOwnedGames"), 2, "a new run serves the copy saved a moment ago");
        assert_eq!(page_of(&served).0.len(), 1);
        rig.clock.advance_s(300);
        library(&mut rig);
        assert_eq!(rig.http.count("GetOwnedGames"), 3);
    }

    fn page_event(events: &[FromEngine]) -> Event {
        match events {
            [FromEngine { event, .. }] => event.clone(),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_refresh_action_makes_the_next_request_go_to_steam_once_and_other_actions_do_nothing() {
        let mut rig = rig_with(&fake::owned(&[(10, "Ten", 1, 1_700_000_000)]));
        let first = page_of(&library(&mut rig));
        rig.clock.advance_s(10);
        assert!(rig.send(Request::Action { key: "refresh".into() }).is_empty(), "the action itself says nothing");
        assert!(rig.send(Request::Action { key: "refresh-apps".into() }).is_empty());
        assert_eq!(rig.http.count("GetOwnedGames"), 1);
        assert_eq!(page_of(&library(&mut rig)), first);
        assert_eq!(rig.http.count("GetOwnedGames"), 2, "10 seconds after a fetch, because the owner asked");
        library(&mut rig);
        assert_eq!(rig.http.count("GetOwnedGames"), 2, "once: the wait is back");

        // Another action is not a refresh.
        rig.send(Request::Action { key: "limits".into() });
        library(&mut rig);
        assert_eq!(rig.http.count("GetOwnedGames"), 2);

        // A refresh that cannot reach Steam serves the saved copy and spends the request.
        rig.http.on("GetOwnedGames", [fake::down()]);
        rig.send(Request::Action { key: "refresh".into() });
        assert_eq!(page_of(&library(&mut rig)), first, "no error: what was saved is served");
        assert_eq!(rig.http.count("GetOwnedGames"), 3);
        library(&mut rig);
        assert_eq!(rig.http.count("GetOwnedGames"), 3);

        // A page of a round already under way is not a start: it neither asks nor uses the request.
        rig.send(Request::Action { key: "refresh".into() });
        assert_eq!(message(&cursor(&mut rig, "200")), (ErrorCode::BadRequest, STALE_CURSOR));
        assert_eq!(rig.http.count("GetOwnedGames"), 3);
    }

    #[test]
    fn when_steam_cannot_be_reached_the_saved_copy_is_served_and_the_next_request_tries_again() {
        let mut rig = rig_with(&fake::owned(&[(10, "Ten", 1, 1_700_000_000)]));
        let first = page_of(&library(&mut rig));
        for failure in [fake::down(), fake::status(500), fake::eresult(3, "{}"), fake::ok("<html>"), fake::ok(r#"{"response":{"games":"x"}}"#)] {
            rig.clock.advance_s(301);
            rig.http.on("GetOwnedGames", [failure]);
            let tries = rig.http.count("GetOwnedGames");
            assert_eq!(page_of(&library(&mut rig)), first, "an old copy beats an error");
            assert_eq!(page_of(&library(&mut rig)), first);
            assert_eq!(rig.http.count("GetOwnedGames"), tries + 2, "a failure does not renew the copy, so it is tried again at once");
        }
        rig.http.on("GetOwnedGames", [fake::ok(&fake::owned(&[(11, "Eleven", 1, 1_700_000_000)]))]);
        assert_eq!(ids(&page_of(&library(&mut rig)).0), ["11"]);
        let events = rig.send(Request::Targets);
        let [FromEngine { event: Event::Targets { targets }, .. }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!(targets[0].link, Link::Linked, "trouble is not an expiry");
    }

    #[test]
    fn with_nothing_saved_trouble_is_an_error_that_says_why() {
        let mut rig = rig_with(&fake::owned(&[]));
        rig.http.on("GetOwnedGames", [fake::down()]);
        assert_eq!(message(&library(&mut rig)), (ErrorCode::Network, NOT_REACHABLE));
        rig.http.on("GetOwnedGames", [fake::ok(&fake::owned(&[(10, "Ten", 0, 0)]))]);
        assert_eq!(ids(&page_of(&library(&mut rig)).0), ["10"], "and the next try is not held back");

        let mut rig = rig_with(&fake::owned(&[]));
        rig.http.on("GenerateAccessTokenForApp", [fake::down()]);
        assert_eq!(message(&library(&mut rig)), (ErrorCode::Network, NOT_REACHABLE));
    }

    #[test]
    fn a_library_steam_refuses_to_give_is_an_expired_sign_in_and_the_saved_copy_still_shows() {
        for refusal in [fake::status(401), fake::status(403), fake::eresult(15, r#"{"response":{}}"#)] {
            let mut rig = rig_with(&fake::owned(&[(10, "Ten", 1, 1_700_000_000)]));
            let first = page_of(&library(&mut rig));
            rig.clock.advance_s(301);
            rig.http.on("GetOwnedGames", [refusal]);
            let events = library(&mut rig);
            let [served, FromEngine { re: None, event: Event::Targets { targets } }] = events.as_slice() else { panic!("{events:?}") };
            assert_eq!(page_of(std::slice::from_ref(served)), first, "an empty body with result 15 does not empty the library");
            assert_eq!(targets[0].link, Link::Expired);
            assert!(rig.cache().join("library.json").exists());
            rig.http.forget();
            assert_eq!(page_of(&library(&mut rig)), first, "still served, and Steam is not asked again");
            assert!(rig.http.sent().is_empty());
        }
        let mut rig = rig_with(&fake::owned(&[]));
        rig.http.on("GetOwnedGames", [fake::status(401)]);
        let events = library(&mut rig);
        assert_eq!(message(&events[..1]), (ErrorCode::SignIn, "The Steam sign-in has expired and nothing is saved yet"));
    }

    #[test]
    fn an_account_with_no_games_is_an_empty_library_and_replaces_what_was_saved() {
        let mut rig = rig_with(&fake::owned(&[(10, "Ten", 1, 1_700_000_000)]));
        library(&mut rig);
        rig.clock.advance_s(301);
        rig.http.on("GetOwnedGames", [fake::ok(r#"{"response":{}}"#)]);
        assert_eq!(page_of(&library(&mut rig)), (Vec::new(), None));
        rig.http.on("GetOwnedGames", [fake::ok(r#"{"response":{"game_count":0}}"#)]);
        rig.clock.advance_s(301);
        assert_eq!(page_of(&library(&mut rig)), (Vec::new(), None));
        assert_eq!(message(&cursor(&mut rig, "200")).1, STALE_CURSOR);
    }

    #[test]
    fn odd_games_are_cleaned_or_dropped() {
        let long = "x".repeat(200);
        let body = format!(
            r#"{{"response":{{"games":[
                {{"appid":10,"name":"First","playtime_forever":3}},
                {{"appid":10,"name":"Second","playtime_forever":9}},
                {{"appid":4294967296,"name":"Too big"}},
                {{"appid":0,"name":"Zero"}},
                {{"appid":11,"name":"Tab\there\u0007 \"quoted\"","playtime_forever":-5,"rtime_last_played":-1}},
                {{"appid":12,"name":"   ","playtime_forever":"soon","rtime_last_played":0}},
                {{"appid":13,"name":"{long}","rtime_last_played":1700000000,"playtime_forever":18446744073709551615}},
                {{"appid":14,"name":7}}
            ]}}}}"#
        );
        let mut rig = rig_with(&body);
        let (entries, _) = page_of(&library(&mut rig));
        assert_eq!(ids(&entries), ["13", "10", "11", "12", "14"]);
        let by = |id: &str| entries.iter().find(|entry| entry.id.as_str() == id).unwrap();
        assert_eq!((by("10").title.as_str(), by("10").activity.playtime_s), ("First", Some(180)), "the first of a repeated game is kept");
        assert_eq!(by("11").title, "Tabhere \"quoted\"");
        assert_eq!((by("11").activity.playtime_s, by("11").activity.last_played), (None, None));
        assert_eq!((by("12").title.as_str(), by("12").activity.playtime_s), ("Steam app 12", None), "a total that is no number is no play time");
        assert_eq!(
            (by("13").title.chars().count(), by("13").activity.playtime_s),
            (160, Some(u64::MAX)),
            "a total too big to count in seconds stops at the most"
        );
        assert_eq!((by("14").title.as_str(), by("14").activity.is_empty()), ("Steam app 14", true));
    }

    #[test]
    fn pages_hold_two_hundred_games_and_each_cursor_leads_to_the_next() {
        let mut rig = rig_with(&many(450));
        rig.http.on("IStoreService", [fake::down()]);
        let (first, next) = page_of(&library(&mut rig));
        assert_eq!((first.len(), ids(&first)[0], ids(&first)[199], next.as_deref()), (200, "1", "200", Some("200")));
        let (second, next) = page_of(&cursor(&mut rig, "200"));
        assert_eq!((second.len(), ids(&second)[0], ids(&second)[199], next.as_deref()), (200, "201", "400", Some("400")));
        let (third, next) = page_of(&cursor(&mut rig, "400"));
        assert_eq!((third.len(), ids(&third)[0], ids(&third)[49], next), (50, "401", "450", None));
        let asked = rig.http.sent().len();
        for bad in ["", "abc", "-200", "+200", "0200", "200 ", " 200", "100", "0", "00", "450", "600", "200.0", "0x64", "99999999999999999999", "١٢٠٠"] {
            assert_eq!(message(&cursor(&mut rig, bad)), (ErrorCode::BadRequest, STALE_CURSOR), "{bad:?}");
        }
        assert_eq!(rig.http.sent().len(), asked, "a cursor never goes to Steam");
        // After a restart a page can be had from the saved copy; with no copy a cursor has nothing to point into.
        rig.restart();
        rig.hello();
        assert_eq!(page_of(&cursor(&mut rig, "400")).0.len(), 50);
        fs::remove_file(rig.cache().join("library.json")).unwrap();
        rig.restart();
        rig.hello();
        assert_eq!(message(&cursor(&mut rig, "200")), (ErrorCode::BadRequest, STALE_CURSOR));
    }

    #[test]
    fn a_library_is_cut_at_fifteen_hundred_games_after_it_is_ordered() {
        let mut rig = rig_with(&many(1_600));
        rig.http.on("IStoreService", [fake::down()]);
        let mut seen = Vec::new();
        let mut round = library(&mut rig);
        loop {
            let (entries, next) = page_of(&round);
            seen.extend(entries.iter().map(|entry| entry.id.as_str().to_owned()));
            let Some(next) = next else { break };
            round = cursor(&mut rig, &next);
        }
        assert_eq!(seen.len(), 1_500);
        assert_eq!((seen[0].as_str(), seen[1_499].as_str()), ("1", "1500"), "the most recently played are the ones kept");
    }

    #[test]
    fn the_longest_page_there_can_be_fits_the_line_limit_with_room_to_spare() {
        // Every name as wide as a name can be, and every game with every kind of tag.
        let wide = "\u{1d538}";
        let all: Vec<u32> = [19, 21, 597, 492, 128, 699, 122, 599, 701, 9].into_iter().chain(1_000..1_008).collect();
        // 64 characters each, the most a name keeps, and all different, because a kind is listed once.
        let listed = all.iter().map(|id| format!(r#"{{"tagid":{id},"name":"{}{id:04}"}}"#, wide.repeat(60))).collect::<Vec<_>>().join(",");
        let tagids = all.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let items = (1..=200).map(|appid| format!(r#"{{"id":{appid},"success":1,"tagids":[{tagids}]}}"#)).collect::<Vec<_>>().join(",");
        let http = FakeHttp::default();
        http.on("IStoreService", [fake::ok(&format!(r#"{{"response":{{"version_hash":"1","tags":[{listed}]}}}}"#))]);
        http.on("IStoreBrowseService", [fake::ok(&format!(r#"{{"response":{{"store_items":[{items}]}}}}"#))]);
        let mut store = Store::default();
        let done = store.refresh(&http, UNIX0, &(1..=200).collect::<Vec<u32>>());
        assert!(done.stopped.is_none());

        let long_title = title(&wide.repeat(200));
        let games: Vec<Game> = (1..=200).map(|appid| Game { appid, name: long_title.clone(), minutes: u64::MAX, last_played: Some(u64::MAX), recent_minutes: 0 }).collect();
        let entries: Vec<LibraryEntry> = games.iter().filter_map(|game| entry(game, &store)).collect();
        assert_eq!((entries[0].activity.genres.len(), entries[0].activity.tags.len()), (6, 5));
        let (shown, next) = page(&entries, None).unwrap();
        assert_eq!((shown.len(), next), (200, None));
        let line = ipc::encode(&FromEngine::answer(u64::MAX, Event::Library { target: target(), entries: shown, next: Some("1400".to_owned()) }));
        assert!(line.len() < ipc::MAX_LINE_BYTES * 3 / 4, "{} bytes", line.len());
    }

    #[test]
    fn kinds_of_game_that_could_not_be_had_are_filled_in_by_a_later_refresh() {
        let mut rig = rig_with(&fake::owned(&[(10, "Ten", 1, 1_700_000_000)]));
        rig.http.on("IStoreBrowseService", [fake::status(500)]);
        let (entries, _) = page_of(&library(&mut rig));
        assert_eq!(ids(&entries), ["10"]);
        assert!(entries[0].activity.genres.is_empty(), "no kinds yet, but the library is served");
        assert!(rig.log.lines().iter().any(|line| line.contains("kinds of game stopped") && line.contains("HTTP 500")), "{:?}", rig.log.lines());

        rig.clock.advance_s(301);
        rig.http.on("IStoreBrowseService", [fake::ok(r#"{"response":{"store_items":[{"id":10,"success":1,"tagids":[19]}]}}"#)]);
        let (entries, _) = page_of(&library(&mut rig));
        assert_eq!(entries[0].activity.genres, ["Action"]);
        assert_eq!(rig.http.count("GetTagList"), 1, "the list of names was had the first time");
    }

    #[test]
    fn a_saved_library_reads_back_equal_and_one_that_is_not_this_accounts_or_not_ours_is_not_served() {
        let mut rig = rig_with(&fake::owned(&[(10, "Ten", 1, 1_700_000_000)]));
        library(&mut rig);
        assert_eq!(Snapshot::load(&rig.cache(), STEAMID).as_ref(), rig.adapter.snapshot.as_ref());
        assert!(rig.adapter.snapshot.is_some());
        let file = fs::read(rig.cache().join("library.json")).unwrap();
        let tampered = |edit: &dyn Fn(&mut Value)| {
            let mut value: Value = serde_json::from_slice(&file).unwrap();
            edit(&mut value);
            serde_json::to_vec(&value).unwrap()
        };
        let too_many = tampered(&|value| value["entries"] = Value::Array(vec![value["entries"][0].clone(); MAX_ENTRIES + 1]));
        let foreign = tampered(&|value| value["steamid"] = "76561198000000002".into());
        let oversize = vec![b' '; usize::try_from(CACHE_MAX).unwrap() + 1];
        for content in [too_many, foreign, b"nonsense".to_vec(), b"{}".to_vec(), oversize] {
            fs::write(rig.cache().join("library.json"), content).unwrap();
            rig.restart();
            rig.hello();
            rig.http.on("GetOwnedGames", [fake::down()]);
            assert_eq!(message(&library(&mut rig)), (ErrorCode::Network, NOT_REACHABLE));
        }
    }

    #[test]
    fn a_saved_library_goes_stale_after_five_minutes_and_when_dated_in_the_future() {
        let snapshot = Snapshot { steamid: STEAMID.to_owned(), fetched_s: 10_000, entries: Vec::new() };
        assert!(!snapshot.stale(10_000) && !snapshot.stale(10_299));
        assert!(snapshot.stale(10_300), "five minutes old");
        assert!(snapshot.stale(9_999), "a clock that was wrong is not trusted");
    }

    #[test]
    fn other_accounts_and_signed_out_requests_are_refused() {
        let mut rig = rig_with(&fake::owned(&[]));
        let other = TargetId::new("76561198000000002").unwrap();
        let events = rig.send(Request::Library { target: other, cursor: None });
        assert_eq!(message(&events), (ErrorCode::BadRequest, "That is not the signed-in Steam account"));
        assert!(rig.http.sent().is_empty());
        let mut rig = Rig::ready();
        assert_eq!(message(&library(&mut rig)), (ErrorCode::BadRequest, "No Steam account is signed in"));
    }
}
