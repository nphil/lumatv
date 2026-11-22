//! What kind of game each game is, from Steam's store: its tags.
//!
//! Two public questions, neither needing a sign-in: what the numbered tags are called, and which tags an app carries, heaviest first. A
//! handful of the tags are what the app calls a genre; the rest are plain tags. The answers are kept in `store.json`, so a refresh asks
//! only about the apps it has not asked about for a month.

use crate::auth::Trouble;
use crate::files;
use crate::http::{self, Http};
use lumagame_core::activity::{MAX_GENRES, genre_label};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::Path;

pub const CACHE_FILE: &str = "store.json";
/// The most of the file that is read: a file left by something else may be any size.
const CACHE_MAX: u64 = 4 << 20;
const MAX_APPS: usize = 4000;
const MAX_TAGS: usize = 1000;
/// Tag ids kept per app, heaviest first. Steam is asked for 8.
const MAX_IDS: usize = 16;
const MAX_PLAIN_TAGS: usize = 5;
const NAME_CHARS: usize = 64;
/// How long a tag list or an app's tags are trusted. Tags drift slowly.
const FRESH_S: u64 = 30 * 86_400;
/// Apps in one question to Steam, and in one refresh: a big library's first refresh is spread over several.
const BATCH: usize = 50;
const BUDGET: usize = 400;
/// The tags Steam files as a genre: Action, Adventure, Casual, Indie, Massively Multiplayer, Racing, RPG, Simulation, Sports, Strategy.
const GENRES: [u32; 10] = [19, 21, 597, 492, 128, 699, 122, 599, 701, 9];

/// What `store.json` holds.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    #[serde(default)]
    tags_version: String,
    /// When the tag list was last confirmed, unix seconds.
    #[serde(default)]
    tags_at: u64,
    #[serde(default)]
    tags: BTreeMap<u32, String>,
    #[serde(default)]
    apps: BTreeMap<u32, AppTags>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AppTags {
    /// When Steam was asked, unix seconds.
    at: u64,
    /// Tag ids, heaviest first. Empty for an app the store does not know.
    ids: Vec<u32>,
}

/// What a refresh did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Refreshed {
    /// Something was learned, so the file is worth saving.
    pub changed: bool,
    /// Apps whose tags came back.
    pub looked_up: usize,
    /// What ended the asking early. Never an error for the library: it only goes without tags until the next refresh.
    pub stopped: Option<Trouble>,
}

/// Whether something fetched at `at` is due again. A time in the future is a clock that was wrong, and is not trusted.
fn stale(at: u64, now: u64) -> bool {
    at > now || now - at >= FRESH_S
}

/// A tag's name for display: one line, single spaces, at most 64 characters.
fn tag_name(raw: &str) -> Option<String> {
    let spaced: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let name: String = spaced.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(NAME_CHARS).collect();
    let name = name.trim_end();
    (!name.is_empty()).then(|| name.to_owned())
}

/// A name as two spellings of it share it: lowercase letters and digits.
fn key(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

impl Store {
    /// What was saved, or nothing when the file is missing, too big or not ours.
    pub fn load(dir: &Path) -> Self {
        let mut store: Self = files::read(dir, CACHE_FILE, CACHE_MAX).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
        store.prune();
        store
    }

    pub fn save(&self, dir: &Path) -> io::Result<()> {
        files::write(dir, CACHE_FILE, &serde_json::to_vec(self).map_err(io::Error::other)?)
    }

    /// Learns the tags of `wanted`, the most wanted first, as far as the budget and Steam allow.
    pub fn refresh(&mut self, http: &impl Http, now: u64, wanted: &[u32]) -> Refreshed {
        let mut done = Refreshed::default();
        if self.tags.is_empty() || stale(self.tags_at, now) {
            match self.fetch_tags(http, now) {
                Ok(()) => done.changed = true,
                Err(trouble) => done.stopped = Some(trouble),
            }
        }
        // Ids mean nothing without their names: the apps wait for a refresh that gets the list.
        if self.tags.is_empty() {
            return done;
        }
        let due: Vec<u32> = wanted.iter().copied().filter(|appid| self.apps.get(appid).is_none_or(|app| stale(app.at, now))).take(BUDGET).collect();
        for batch in due.chunks(BATCH) {
            match fetch_items(http, batch) {
                Ok(found) => {
                    done.looked_up += found.len();
                    done.changed |= !found.is_empty();
                    self.apps.extend(found.into_iter().map(|(appid, ids)| (appid, AppTags { at: now, ids })));
                }
                Err(trouble) => {
                    done.stopped = Some(trouble);
                    break;
                }
            }
        }
        if done.changed {
            self.prune();
        }
        done
    }

    fn fetch_tags(&mut self, http: &impl Http, now: u64) -> Result<(), Trouble> {
        let resp = http.get(&http::api("/IStoreService/GetTagList/v1/?language=english"), http::SMALL_BODY).map_err(Trouble::Network)?;
        if !resp.succeeded() {
            return Err(Trouble::of(&resp));
        }
        let (version, tags) = read_tag_list(&resp.body).ok_or(Trouble::Unreadable)?;
        // The same version is the same list: only the date is renewed.
        if version.is_empty() || version != self.tags_version || self.tags.is_empty() {
            self.tags_version = version;
            self.tags = tags;
        }
        self.tags_at = now;
        Ok(())
    }

    /// An app's genres and its other tags, each named once. Empty for an app that is not known yet.
    pub fn classify(&self, appid: u32) -> (Vec<String>, Vec<String>) {
        let Some(app) = self.apps.get(&appid) else { return (Vec::new(), Vec::new()) };
        let named = |id: &u32| self.tags.get(id);
        let mut seen = HashSet::new();
        let genres = app
            .ids
            .iter()
            .filter(|id| GENRES.contains(id))
            .filter_map(|id| named(id).and_then(|name| genre_label(name)))
            .filter(|label| seen.insert(key(label)))
            .take(MAX_GENRES)
            .collect();
        let tags =
            app.ids.iter().filter(|id| !GENRES.contains(id)).filter_map(named).filter(|name| seen.insert(key(name))).take(MAX_PLAIN_TAGS).cloned().collect();
        (genres, tags)
    }

    /// Keeps the file to its bounds: what was read from disk may be anything.
    fn prune(&mut self) {
        self.tags = std::mem::take(&mut self.tags).into_iter().filter_map(|(id, name)| Some((id, tag_name(&name)?))).take(MAX_TAGS).collect();
        self.tags_version = self.tags_version.chars().take(NAME_CHARS).collect();
        for app in self.apps.values_mut() {
            app.ids.truncate(MAX_IDS);
        }
        let excess = self.apps.len().saturating_sub(MAX_APPS);
        if excess > 0 {
            let mut oldest: Vec<(u64, u32)> = self.apps.iter().map(|(appid, app)| (app.at, *appid)).collect();
            oldest.sort_unstable();
            for (_, appid) in oldest.into_iter().take(excess) {
                self.apps.remove(&appid);
            }
        }
    }
}

fn read_tag_list(body: &[u8]) -> Option<(String, BTreeMap<u32, String>)> {
    #[derive(Deserialize)]
    struct Body {
        response: Fields,
    }
    #[derive(Deserialize)]
    struct Fields {
        version_hash: Option<Value>,
        tags: Vec<Tag>,
    }
    #[derive(Deserialize)]
    struct Tag {
        tagid: u32,
        name: String,
    }
    let Body { response } = serde_json::from_slice(body).ok()?;
    let tags: BTreeMap<u32, String> = response.tags.into_iter().filter_map(|tag| Some((tag.tagid, tag_name(&tag.name)?))).collect();
    // Steam sends the version as a string of digits; a number is read the same.
    let version = match response.version_hash {
        Some(Value::String(text)) => text,
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    };
    // An empty list would replace a good one with nothing.
    (!tags.is_empty()).then_some((version, tags))
}

/// The tag ids of each of `batch` that the answer covers. An app missing from the answer is left out, to be asked about again.
fn fetch_items(http: &impl Http, batch: &[u32]) -> Result<BTreeMap<u32, Vec<u32>>, Trouble> {
    let ids = batch.iter().map(|appid| format!(r#"{{"appid":{appid}}}"#)).collect::<Vec<_>>().join(",");
    let input = format!(r#"{{"ids":[{ids}],"context":{{"language":"english","country_code":"US"}},"data_request":{{"include_tag_count":8}}}}"#);
    let url = http::api(&format!("/IStoreBrowseService/GetItems/v1/?input_json={}", http::percent_encode(&input)));
    let resp = http.get(&url, http::SMALL_BODY).map_err(Trouble::Network)?;
    if !resp.succeeded() {
        return Err(Trouble::of(&resp));
    }
    #[derive(Deserialize)]
    struct Body {
        response: Fields,
    }
    #[derive(Deserialize)]
    struct Fields {
        #[serde(default)]
        store_items: Vec<Item>,
    }
    #[derive(Deserialize)]
    struct Item {
        id: u64,
        success: Option<i64>,
        tagids: Option<Vec<u32>>,
    }
    let Body { response } = serde_json::from_slice(&resp.body).map_err(|_| Trouble::Unreadable)?;
    let mut found = BTreeMap::new();
    for item in response.store_items {
        // By `id`: an app the store does not know comes back with an `appid` of 0.
        let Some(appid) = u32::try_from(item.id).ok().filter(|appid| batch.contains(appid)) else { continue };
        let ids = if item.success == Some(1) { item.tagids.unwrap_or_default().into_iter().take(MAX_IDS).collect() } else { Vec::new() };
        found.insert(appid, ids);
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{self, FakeHttp, TempDir};
    use crate::http::HttpError;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const NOW: u64 = 1_800_000_000;
    const DAY: u64 = 86_400;
    /// The tags of an app that is only a strategy game.
    const STRATEGY: &[u32] = &[9];
    const TAGS: &str = r#"{"response":{"version_hash":"711684454","tags":[{"tagid":9,"name":"Strategy"},{"tagid":19,"name":"Action"},{"tagid":122,"name":"Role-Playing"},{"tagid":492,"name":"Indie"},{"tagid":1695,"name":"Open World"},{"tagid":4604,"name":"Dark \t Fantasy"},{"tagid":29482,"name":"Souls-like"}]}}"#;

    /// What the store answers for `(id, success, tag ids)`.
    fn items(rows: &[(u64, i64, &[u32])]) -> String {
        let rows: Vec<String> = rows
            .iter()
            .map(|(id, success, tagids)| {
                let tagids = tagids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
                format!(r#"{{"item_type":0,"id":{id},"success":{success},"visible":{},"appid":{id},"tagids":[{tagids}]}}"#, *success == 1)
            })
            .collect();
        format!(r#"{{"response":{{"store_items":[{}]}}}}"#, rows.join(","))
    }

    fn store_with(ids: &[(u32, &[u32])]) -> Store {
        let tags = [(9, "Strategy"), (19, "Action"), (122, "Role-Playing"), (1695, "Open World"), (4604, "Dark Fantasy"), (29482, "Souls-like")];
        Store {
            tags: tags.into_iter().map(|(id, name)| (id, name.to_owned())).collect(),
            apps: ids.iter().map(|(appid, ids)| (*appid, AppTags { at: NOW, ids: ids.to_vec() })).collect(),
            ..Store::default()
        }
    }

    fn scripted() -> FakeHttp {
        let http = FakeHttp::default();
        http.on("IStoreService", [fake::ok(TAGS)]);
        http.on("IStoreBrowseService", [fake::ok(&items(&[(10, 1, &[29482, 122, 1695, 19]), (20, 1, &[9])]))]);
        http
    }

    #[test]
    fn the_names_and_the_tags_of_the_wanted_apps_are_asked_for_in_the_documented_form() {
        let http = scripted();
        let mut store = Store::default();
        let done = store.refresh(&http, NOW, &[10, 20]);
        assert_eq!(done, Refreshed { changed: true, looked_up: 2, stopped: None });
        let tag_list = http.to("GetTagList");
        assert_eq!((tag_list.len(), tag_list[0].post), (1, false));
        assert_eq!(tag_list[0].url, "https://api.steampowered.com/IStoreService/GetTagList/v1/?language=english");
        let asked = http.to("GetItems");
        assert_eq!(asked.len(), 1);
        assert_eq!(
            asked[0].url,
            "https://api.steampowered.com/IStoreBrowseService/GetItems/v1/?input_json=%7B%22ids%22%3A%5B%7B%22appid%22%3A10%7D%2C%7B%22appid%22%3A20%7D%5D%2C\
             %22context%22%3A%7B%22language%22%3A%22english%22%2C%22country_code%22%3A%22US%22%7D%2C%22data_request%22%3A%7B%22include_tag_count%22%3A8%7D%7D"
        );
        assert_eq!((store.tags.len(), store.tags[&4604].as_str()), (7, "Dark Fantasy"), "a name is one line with single spaces");
        assert_eq!(store.apps[&10], AppTags { at: NOW, ids: vec![29482, 122, 1695, 19] });
    }

    #[test]
    fn a_tag_list_and_an_apps_tags_are_kept_for_a_month_and_the_list_is_kept_while_its_version_holds() {
        let http = scripted();
        let mut store = Store::default();
        store.refresh(&http, NOW, &[10, 20]);
        http.forget();
        assert_eq!(store.refresh(&http, NOW + 29 * DAY, &[10, 20]), Refreshed::default());
        assert!(http.sent().is_empty(), "nothing is due inside the month");

        let before = store.tags.clone();
        assert!(store.refresh(&http, NOW + 31 * DAY, &[10, 20]).changed);
        assert_eq!((http.count("GetTagList"), http.count("GetItems")), (1, 1));
        assert_eq!((store.tags_at, &store.tags), (NOW + 31 * DAY, &before), "the same version: the list stays and only its date is renewed");
        assert_eq!(store.apps[&10].at, NOW + 31 * DAY);

        let renamed = TAGS.replace("711684454", "9").replace("Open World", "Sandbox");
        http.on("IStoreService", [fake::ok(&renamed)]);
        store.refresh(&http, NOW + 62 * DAY, &[]);
        assert_eq!((store.tags_version.as_str(), store.tags[&1695].as_str()), ("9", "Sandbox"), "a new version replaces the list");
    }

    #[test]
    fn a_version_sent_as_a_number_is_read_and_a_missing_or_empty_list_is_refused() {
        let http = scripted();
        http.on("IStoreService", [fake::ok(&TAGS.replace(r#""711684454""#, "711684454"))]);
        let mut store = Store::default();
        store.refresh(&http, NOW, &[]);
        assert_eq!(store.tags_version, "711684454");

        for body in [r#"{"response":{"version_hash":"1","tags":[]}}"#, r#"{"response":{"version_hash":"1"}}"#, "[]", "nonsense"] {
            http.on("IStoreService", [fake::ok(body)]);
            let mut store = Store::default();
            let done = store.refresh(&http, NOW, &[10]);
            assert_eq!(done, Refreshed { stopped: Some(Trouble::Unreadable), ..Refreshed::default() }, "{body}");
            assert!(store.tags.is_empty());
        }
        // A list that cannot be read does not replace the one that is kept.
        store.refresh(&http, NOW + 40 * DAY, &[]);
        assert_eq!(store.tags.len(), 7);
    }

    #[test]
    fn without_the_list_no_app_is_asked_about_and_with_a_stale_one_they_still_are() {
        let http = FakeHttp::default();
        http.on("IStoreService", [Err(HttpError::Unreachable)]);
        let mut store = Store::default();
        let done = store.refresh(&http, NOW, &[10]);
        assert_eq!(done, Refreshed { changed: false, looked_up: 0, stopped: Some(Trouble::Network(HttpError::Unreachable)) });
        assert_eq!(http.sent().len(), 1, "no names, so no point asking for ids");

        let http = scripted();
        let mut store = store_with(&[]);
        store.tags_at = NOW - 40 * DAY;
        http.on("IStoreService", [fake::status(503)]);
        let done = store.refresh(&http, NOW, &[10]);
        assert_eq!((done.looked_up, done.stopped), (1, Some(Trouble::Status(503))));
        assert_eq!(store.tags.len(), 6, "the old list stays and still names tags");
        assert_eq!(store.apps[&10].ids, [29482, 122, 1695, 19]);
    }

    #[test]
    fn apps_go_to_steam_fifty_at_a_time_four_hundred_to_a_refresh_and_the_rest_next_time() {
        let http = FakeHttp::default();
        http.on("IStoreService", [fake::ok(TAGS)]);
        let wanted: Vec<u32> = (1..=450).collect();
        http.on("IStoreBrowseService", [fake::ok(&items(&[]))]);
        let mut store = Store::default();
        let done = store.refresh(&http, NOW, &wanted);
        assert_eq!(http.count("GetItems"), 8, "400 apps in calls of 50");
        assert_eq!((done.looked_up, store.apps.len()), (0, 0), "apps the answer leaves out are not remembered");

        // Answer for the apps asked about, as the store does.
        let all: Vec<(u64, i64, &[u32])> = (1..=450).map(|id| (id, 1, STRATEGY)).collect();
        http.on("IStoreBrowseService", [fake::ok(&items(&all))]);
        http.forget();
        let done = store.refresh(&http, NOW, &wanted);
        assert_eq!((http.count("GetItems"), done.looked_up, store.apps.len()), (8, 400, 400));
        assert!(wanted[..400].iter().all(|appid| store.apps.contains_key(appid)), "the most wanted come first");
        http.forget();
        let done = store.refresh(&http, NOW + 60, &wanted);
        assert_eq!((http.count("GetItems"), done.looked_up, store.apps.len()), (1, 50, 450));
        assert_eq!(store.refresh(&http, NOW + 120, &wanted), Refreshed::default());
    }

    #[test]
    fn what_the_store_does_not_know_is_remembered_so_that_it_is_not_asked_again() {
        let http = scripted();
        // `appid` is 0 for an unknown app: the answer is read by `id`. 30 is not in the answer, and 99 was never asked.
        let answer = r#"{"response":{"store_items":[
            {"id":10,"success":1,"visible":true,"appid":10,"tagids":[9]},
            {"id":20,"success":15,"visible":false,"name":"","appid":0},
            {"id":40,"success":1,"visible":false,"appid":40,"tagids":[19, 122]},
            {"id":99,"success":1,"visible":true,"appid":99,"tagids":[9]}]}}"#;
        http.on("IStoreBrowseService", [fake::ok(answer)]);
        let mut store = Store::default();
        let done = store.refresh(&http, NOW, &[10, 20, 30, 40]);
        assert_eq!(done.looked_up, 3);
        let ids = |appid: u32| store.apps.get(&appid).map(|app| app.ids.clone());
        assert_eq!((ids(10), ids(20), ids(30), ids(40), ids(99)), (Some(vec![9]), Some(vec![]), None, Some(vec![19, 122]), None));
        http.forget();
        store.refresh(&http, NOW + DAY, &[10, 20, 30, 40]);
        let asked = http.to("GetItems");
        assert_eq!(asked.len(), 1);
        assert!(asked[0].url.contains("%7B%22appid%22%3A30%7D%5D"), "only 30 is asked again: {}", asked[0].url);
    }

    #[test]
    fn asking_stops_at_the_first_trouble_and_keeps_what_arrived() {
        let http = FakeHttp::default();
        http.on("IStoreService", [fake::ok(TAGS)]);
        let first: Vec<(u64, i64, &[u32])> = (1..=50).map(|id| (id, 1, STRATEGY)).collect();
        http.on("IStoreBrowseService", [fake::ok(&items(&first)), fake::eresult(3, "{}")]);
        let wanted: Vec<u32> = (1..=120).collect();
        let mut store = Store::default();
        let done = store.refresh(&http, NOW, &wanted);
        assert_eq!(http.count("GetItems"), 2, "the third batch is not tried");
        assert_eq!(done, Refreshed { changed: true, looked_up: 50, stopped: Some(Trouble::Eresult(Some(3))) });
        assert_eq!(store.apps.len(), 50);
    }

    #[test]
    fn an_unreadable_answer_is_trouble_and_changes_nothing() {
        for reply in [fake::ok("<html>"), fake::ok(r#"{"response":{"store_items":[{"success":1}]}}"#), fake::status(500), fake::down()] {
            let http = FakeHttp::default();
            http.on("IStoreService", [fake::ok(TAGS)]);
            http.on("IStoreBrowseService", [reply]);
            let mut store = Store::default();
            let done = store.refresh(&http, NOW, &[10]);
            assert!(done.stopped.is_some() && done.looked_up == 0 && store.apps.is_empty());
        }
    }

    #[test]
    fn genres_and_tags_are_named_in_weight_order_and_each_once() {
        let mut store = store_with(&[(1, &[29482, 122, 1695, 19, 777, 4604, 9])]);
        let (genres, tags) = store.classify(1);
        assert_eq!(genres, ["RPG", "Action", "Strategy"], "the list's `Role-Playing` is the app's `RPG`");
        assert_eq!(tags, ["Souls-like", "Open World", "Dark Fantasy"], "777 has no name and is skipped");
        assert_eq!(store.classify(2), (Vec::new(), Vec::new()), "an app not looked up yet has nothing");

        // The same word under two ids is shown once, and a plain tag that repeats a genre is dropped.
        store.tags.insert(6000, "rpg".to_owned());
        store.tags.insert(6001, "Open  World".to_owned());
        store.apps.insert(2, AppTags { at: NOW, ids: vec![122, 6000, 1695, 6001, 19, 19] });
        assert_eq!(store.classify(2), (vec!["RPG".to_owned(), "Action".to_owned()], vec!["Open World".to_owned()]));
    }

    #[test]
    fn at_most_six_genres_and_five_other_tags_are_kept() {
        let mut store = Store::default();
        for (id, name) in [
            (19, "Action"),
            (21, "Adventure"),
            (597, "Casual"),
            (492, "Indie"),
            (128, "Massively Multiplayer"),
            (699, "Racing"),
            (122, "RPG"),
            (599, "Simulation"),
        ] {
            store.tags.insert(id, name.to_owned());
        }
        for id in 1..=9 {
            store.tags.insert(id + 10_000, format!("Tag {id}"));
        }
        let mut ids = vec![19, 21, 597, 492, 128, 699, 122, 599];
        ids.extend(10_001..=10_009);
        store.apps.insert(1, AppTags { at: NOW, ids });
        let (genres, tags) = store.classify(1);
        assert_eq!(genres, ["Action", "Adventure", "Casual", "Indie", "Massively Multiplayer", "Racing"]);
        assert_eq!(tags, ["Tag 1", "Tag 2", "Tag 3", "Tag 4", "Tag 5"]);
    }

    #[test]
    fn the_file_round_trips_privately_and_is_kept_within_bounds() {
        let temp = TempDir::new();
        let store = store_with(&[(1, &[9, 19]), (2, &[])]);
        store.save(temp.path()).unwrap();
        assert_eq!(fs::metadata(temp.path().join(CACHE_FILE)).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(Store::load(temp.path()), store);
        let text = fs::read_to_string(temp.path().join(CACHE_FILE)).unwrap();
        assert!(text.contains(r#""apps":{"1":{"at":1800000000,"ids":[9,19]}"#) && text.contains(r#""tags":{"9":"Strategy""#), "{text}");

        let big = Store {
            tags: (0..1_100).map(|id| (id, format!("Tag {id}"))).collect(),
            apps: (0..4_100).map(|appid| (appid, AppTags { at: 5_000 - u64::from(appid % 4_000), ids: (0..20).collect() })).collect(),
            ..Store::default()
        };
        big.save(temp.path()).unwrap();
        let loaded = Store::load(temp.path());
        assert_eq!((loaded.tags.len(), loaded.apps.len()), (1_000, 4_000));
        assert!(loaded.tags.contains_key(&999) && !loaded.tags.contains_key(&1_000));
        assert!(loaded.apps.values().all(|app| app.ids.len() == 16));
        // The 100 oldest go: the apps saved at 1_001 to 1_100.
        let oldest = (0..4_100u32).filter(|appid| !loaded.apps.contains_key(appid)).collect::<Vec<_>>();
        assert_eq!(oldest.len(), 100);
        assert!(oldest.iter().all(|appid| 5_000 - u64::from(appid % 4_000) <= 1_100), "{oldest:?}");
    }

    #[test]
    fn a_missing_corrupt_oversized_or_foreign_file_is_an_empty_store() {
        let temp = TempDir::new();
        assert_eq!(Store::load(temp.path()), Store::default());
        for content in [&b"nonsense"[..], b"[]", br#"{"apps":{"x":{}}}"#, br#"{"tags":{"1":5}}"#] {
            files::write(temp.path(), CACHE_FILE, content).unwrap();
            assert_eq!(Store::load(temp.path()), Store::default());
        }
        fs::write(temp.path().join(CACHE_FILE), vec![b' '; usize::try_from(CACHE_MAX).unwrap() + 1]).unwrap();
        assert_eq!(Store::load(temp.path()), Store::default());
        // Missing fields are the empty ones, so an older file still loads.
        files::write(temp.path(), CACHE_FILE, br#"{"tags_version":"1","tags":{"9":"Strategy"}}"#).unwrap();
        assert_eq!(Store::load(temp.path()).tags.len(), 1);
    }

    #[test]
    fn a_date_in_the_future_is_not_trusted() {
        assert!(stale(NOW + 1, NOW) && stale(NOW - FRESH_S, NOW) && !stale(NOW - FRESH_S + 1, NOW) && !stale(NOW, NOW));
    }

    #[test]
    fn names_are_one_clean_line() {
        assert_eq!(tag_name("  Dark\t\n Fantasy\u{7} ").as_deref(), Some("Dark Fantasy"));
        assert_eq!(tag_name(&"é".repeat(100)).map(|name| name.chars().count()), Some(NAME_CHARS));
        assert_eq!((tag_name(""), tag_name(" \t ")), (None, None));
    }
}
