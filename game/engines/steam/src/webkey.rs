//! Linking by Steam Web API key: no sign-in session at all.
//!
//! The owner makes a key for their own account on Steam's website and types it in with the account's id. Steam answers the key's
//! read-only calls for that account's public data: the owned games with play time and last played, recent play, the profile name and
//! picture, and the owner's achievements in a game. Nothing here renews, so a password change or a revoked session cannot end it; only
//! Steam withdrawing the key does, and then the account is shown as needing a new key. What a key cannot read, friends included, is
//! not offered.
//!
//! The key travels as the `key` query parameter, the form every Steam endpoint takes. No address built here is ever logged, and a
//! request that carries the key is never repeated after Steam refuses it: refusals of a key count against the connection's address.

use crate::auth::{Refusal, Trouble, clean_name};
use crate::http::{self, Http, HttpError, Resp};
use crate::{Adapter, Clock, files};
use lumagame_core::ids::TargetId;
use lumagame_core::ipc::{AuthFail, AuthState, Event, Secret};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::Path;

pub const KEY_FILE: &str = "apikey.json";
/// What the app shows beside the account's name.
pub const DETAIL: &str = "Web API key";
/// After Steam answers "too many requests", the key is left alone this long.
const PAUSE_S: u64 = 900;
/// The first and one past the last SteamID64 of an individual account.
const ID_FROM: u64 = 76_561_197_960_265_728;
const ID_TO: u64 = 76_561_202_255_233_024;

/// A key and the account it reads. It has no `Debug`: the key is a credential.
pub struct KeyAccount {
    key: String,
    /// The account's SteamID64 in decimal, which is also its target id.
    pub steamid: String,
    pub persona: Option<String>,
    /// Steam refused the key since the engine started; the file is kept and the next start tries once more.
    pub refused: bool,
    /// No request carrying the key before this unix time, after Steam asked for a pause.
    paused_until: u64,
}

#[derive(Deserialize)]
struct FileIn {
    key: String,
    steamid: String,
    #[serde(default)]
    persona: Option<String>,
}

#[derive(Serialize)]
struct FileOut<'a> {
    key: &'a str,
    steamid: &'a str,
    persona: Option<&'a str>,
}

impl KeyAccount {
    pub fn load(dir: &Path) -> Option<Self> {
        let file: FileIn = serde_json::from_slice(&files::read(dir, KEY_FILE, 4 << 10)?).ok()?;
        let key = normalise_key(&file.key)?;
        let who = parse_who(&file.steamid)?;
        let Who::Id(steamid) = who else { return None };
        Some(Self { key, steamid, persona: file.persona.as_deref().and_then(clean_name), refused: false, paused_until: 0 })
    }

    fn save(&self, dir: &Path) -> io::Result<()> {
        let file = FileOut { key: &self.key, steamid: &self.steamid, persona: self.persona.as_deref() };
        files::write(dir, KEY_FILE, &serde_json::to_vec(&file).map_err(io::Error::other)?)
    }

    pub fn target_id(&self) -> TargetId {
        TargetId::new(self.steamid.clone()).expect("a Steam id is a valid target id")
    }
}

/// A Web API key as Steam writes it: 32 hexadecimal digits. Spaces between groups are dropped; the result is upper case.
pub fn normalise_key(text: &str) -> Option<String> {
    let key: String = text.chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    (key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit())).then(|| key.to_ascii_uppercase())
}

/// What the owner typed to name the account.
#[derive(Debug, PartialEq, Eq)]
pub enum Who {
    Id(String),
    /// The custom part of a profile address, which Steam turns into an id.
    Vanity(String),
}

/// A SteamID64, a community profile address (`/profiles/<id>` or `/id/<name>`), or a profile name.
pub fn parse_who(text: &str) -> Option<Who> {
    let text = text.trim();
    let id = |digits: &str| {
        let number = digits.parse::<u64>().ok().filter(|n| digits.len() == 17 && (ID_FROM..ID_TO).contains(n))?;
        Some(Who::Id(number.to_string()))
    };
    if text.bytes().all(|b| b.is_ascii_digit()) {
        return id(text);
    }
    let name = |name: &str| (2..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if let Some(rest) = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://")) {
        let (host, path) = rest.split_once('/')?;
        if !matches!(host, "steamcommunity.com" | "www.steamcommunity.com") {
            return None;
        }
        let path = path.split(['?', '#']).next()?;
        let mut parts = path.split('/').filter(|part| !part.is_empty());
        return match (parts.next()?, parts.next()?) {
            ("profiles", digits) => id(digits),
            ("id", vanity) if name(vanity) => Some(Who::Vanity(vanity.to_owned())),
            _ => None,
        };
    }
    name(text).then(|| Who::Vanity(text.to_owned()))
}

/// Why a call with the key gave nothing to use.
#[derive(Debug, PartialEq, Eq)]
enum Fault {
    /// Steam does not accept the key.
    Refused,
    /// Steam asked for a pause.
    Limited,
    Trouble(Trouble),
}

impl Fault {
    fn fail(&self) -> (AuthFail, String) {
        match self {
            Self::Refused => (AuthFail::BadKey, "Steam did not accept this key. Check that all 32 characters were typed.".to_owned()),
            Self::Limited => (AuthFail::Network, "Steam asked for a pause. Try again in a few minutes.".to_owned()),
            Self::Trouble(trouble) => (AuthFail::Network, trouble.to_string()),
        }
    }
}

/// Steam turns a key it rejects away with a page of HTML under 401 or 403. A reply that is JSON is the data endpoint's own answer, such
/// as "this profile is not public", and says nothing about the key.
pub fn key_refused(resp: &Resp) -> bool {
    matches!(resp.status, 401 | 403) && !resp.body.iter().find(|b| !b.is_ascii_whitespace()).is_some_and(|b| *b == b'{')
}

/// The profile Steam knows for an id.
struct Profile {
    persona: Option<String>,
    avatar: Option<String>,
}

fn parse_profile(body: &[u8]) -> Option<Option<Profile>> {
    #[derive(Deserialize)]
    struct Body {
        response: Fields,
    }
    #[derive(Deserialize)]
    struct Fields {
        #[serde(default)]
        players: Vec<Player>,
    }
    #[derive(Deserialize)]
    struct Player {
        personaname: Option<String>,
        avatarmedium: Option<String>,
    }
    let Body { response } = serde_json::from_slice(body).ok()?;
    Some(response.players.into_iter().next().map(|player| Profile {
        persona: player.personaname.as_deref().and_then(clean_name),
        avatar: player.avatarmedium.and_then(|url| crate::avatar::steam_image(&url)),
    }))
}

fn parse_vanity(body: &[u8]) -> Option<Result<String, ()>> {
    #[derive(Deserialize)]
    struct Body {
        response: Fields,
    }
    #[derive(Deserialize)]
    struct Fields {
        success: Option<u32>,
        steamid: Option<String>,
    }
    let Body { response } = serde_json::from_slice(body).ok()?;
    match (response.success, response.steamid) {
        (Some(1), Some(id)) => match parse_who(&id) {
            Some(Who::Id(id)) => Some(Ok(id)),
            _ => None,
        },
        (Some(42), _) => Some(Err(())),
        _ => None,
    }
}

/// Whether a `GetOwnedGames` answer is the empty object Steam gives when it hides the games: an account with none has a count.
pub fn games_hidden(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body).ok().and_then(|v| v.get("response")?.as_object().map(|r| !r.contains_key("games") && !r.contains_key("game_count"))).unwrap_or(false)
}

impl<H: Http, C: Clock> Adapter<H, C> {
    /// One read of the web API with `key`. A rejected key and a request for a pause are told apart; every other answer is the caller's.
    fn call(&self, key: &str, path: &str, pairs: &[(&str, &str)], limit: usize) -> Result<Resp, Fault> {
        let mut all = vec![("key", key)];
        all.extend_from_slice(pairs);
        all.push(("format", "json"));
        let resp = self.http.get(&http::api(&format!("{path}?{}", http::form(&all))), limit).map_err(|error: HttpError| Fault::Trouble(Trouble::Network(error)))?;
        if key_refused(&resp) {
            return Err(Fault::Refused);
        }
        if resp.status == 429 {
            return Err(Fault::Limited);
        }
        Ok(resp)
    }

    /// Whether reads go by the key: there is one and Steam has not refused it since the engine started.
    pub(crate) fn reads_with_key(&self) -> bool {
        self.key.as_ref().is_some_and(|key| !key.refused)
    }

    /// A read of the web API with the saved key, for an account that has one. A refusal is remembered, so nothing more is sent with it.
    pub(crate) fn read_with_key(&mut self, path: &str, pairs: &[(&str, &str)], limit: usize) -> Result<Resp, Refusal> {
        let now = self.clock.unix_s();
        let Some(key) = self.key.as_ref().filter(|key| !key.refused) else { return Err(Refusal::Key) };
        if now < key.paused_until {
            return Err(Refusal::Trouble(Trouble::Status(429)));
        }
        let key = key.key.clone();
        match self.call(&key, path, pairs, limit) {
            Ok(resp) => Ok(resp),
            Err(Fault::Refused) => {
                if let Some(key) = self.key.as_mut() {
                    key.refused = true;
                }
                self.targets_changed();
                self.note("web key: Steam does not accept the key");
                Err(Refusal::Key)
            }
            Err(Fault::Limited) => {
                if let Some(key) = self.key.as_mut() {
                    key.paused_until = now + PAUSE_S;
                }
                self.note("web key: Steam asked for a pause");
                Err(Refusal::Trouble(Trouble::Status(429)))
            }
            Err(Fault::Trouble(trouble)) => Err(Refusal::Trouble(trouble)),
        }
    }

    /// Links the account named by `who` for the key `key`, as `auth-key` asks. The key is checked with Steam before anything is kept.
    pub(crate) fn link_with_key(&mut self, seq: u64, key: &Secret, who: &str) {
        let Some(key) = normalise_key(key.expose()) else {
            return self.fail_key(seq, AuthFail::BadKey, "A Web API key is 32 characters: the digits 0 to 9 and the letters A to F.");
        };
        let Some(who) = parse_who(who) else {
            return self.fail_key(seq, AuthFail::BadKey, "Use the 17-digit Steam ID, the profile address or the profile name.");
        };
        let steamid = match who {
            Who::Id(id) => id,
            Who::Vanity(name) => match self.call(&key, "/ISteamUser/ResolveVanityURL/v1/", &[("vanityurl", &name)], http::SMALL_BODY) {
                Ok(resp) if resp.status == 200 => match parse_vanity(&resp.body) {
                    Some(Ok(id)) => id,
                    Some(Err(())) => return self.fail_key(seq, AuthFail::BadKey, "No Steam profile has that name."),
                    None => return self.fail_key(seq, AuthFail::Network, &Trouble::Unreadable.to_string()),
                },
                Ok(resp) => return self.fail_key(seq, AuthFail::Network, &Trouble::of(&resp).to_string()),
                Err(fault) => return self.fail_with(seq, &fault),
            },
        };
        let profile = match self.call(&key, "/ISteamUser/GetPlayerSummaries/v2/", &[("steamids", &steamid)], http::SMALL_BODY) {
            Ok(resp) if resp.status == 200 => match parse_profile(&resp.body) {
                Some(Some(profile)) => profile,
                Some(None) => return self.fail_key(seq, AuthFail::BadKey, "Steam has no profile with that ID."),
                None => return self.fail_key(seq, AuthFail::Network, &Trouble::Unreadable.to_string()),
            },
            Ok(resp) => return self.fail_key(seq, AuthFail::Network, &Trouble::of(&resp).to_string()),
            Err(fault) => return self.fail_with(seq, &fault),
        };

        let account = KeyAccount { key, steamid, persona: profile.persona, refused: false, paused_until: 0 };
        if let Err(error) = account.save(&self.data) {
            self.note(&format!("web key: the key could not be saved ({:?})", error.kind()));
            return self.fail_key(seq, AuthFail::Network, "The key could not be saved on the TV");
        }
        // A saved library and picture belong to the account they came from.
        if self.steamid().as_deref() != Some(account.steamid.as_str()) {
            self.snapshot = None;
            self.forget_avatar();
            if let Err(error) = files::remove(&self.cache, crate::library::CACHE_FILE) {
                self.note(&format!("web key: the earlier account's library could not be deleted ({:?})", error.kind()));
            }
        }
        // The newest link wins: a sign-in for another account goes.
        if self.account.as_ref().is_some_and(|session| session.steamid != account.steamid) {
            if let Err(error) = files::remove(&self.data, crate::auth::TOKEN_FILE) {
                self.note(&format!("web key: the earlier sign-in could not be deleted ({:?})", error.kind()));
            }
            self.account = None;
        }
        let target = account.target_id();
        self.key = Some(account);
        if let Some(url) = profile.avatar {
            self.keep_picture(&url);
        }
        self.answer(seq, Event::Auth { state: AuthState::Done { target } });
        self.targets_changed();
        self.note("web key: linked");
    }

    fn fail_with(&mut self, seq: u64, fault: &Fault) {
        let (reason, message) = fault.fail();
        self.fail_key(seq, reason, &message);
    }

    fn fail_key(&mut self, seq: u64, reason: AuthFail, message: &str) {
        self.note(&format!("web key: not linked ({})", reason.as_str()));
        self.answer(seq, Event::Auth { state: AuthState::Failed { reason, message: message.to_owned() } });
    }

    /// The profile name and picture of the key's account, looked up again. Trouble leaves what was kept as it was.
    pub(crate) fn refresh_profile_by_key(&mut self) {
        let Some(steamid) = self.key.as_ref().filter(|key| !key.refused).map(|key| key.steamid.clone()) else { return };
        let Ok(resp) = self.read_with_key("/ISteamUser/GetPlayerSummaries/v2/", &[("steamids", &steamid)], http::SMALL_BODY) else { return };
        let Some(Some(profile)) = (resp.status == 200).then(|| parse_profile(&resp.body)).flatten() else { return };
        if let Some(key) = self.key.as_mut()
            && profile.persona.is_some()
            && key.persona != profile.persona
        {
            key.persona = profile.persona;
            if let Err(error) = key.save(&self.data) {
                self.note(&format!("web key: the profile name could not be saved ({:?})", error.kind()));
            }
            self.targets_changed();
        }
        if let Some(url) = profile.avatar {
            self.keep_picture(&url);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{self, Rig, STEAMID};
    use lumagame_core::ipc::{self, ErrorCode, FromEngine, Link, Request, Target};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    const KEY: &str = "0123456789ABCDEF0123456789ABCDEF";
    const OTHER: &str = "76561198000000002";
    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];
    const FORBIDDEN: &str = "<html><head><title>Forbidden</title></head><body><h1>Forbidden</h1>Access is denied.</body></html>";
    const PRIVATE: &str = r#"{"playerstats":{"error":"Profile is not public","success":false}}"#;

    fn target() -> TargetId {
        TargetId::new(STEAMID).unwrap()
    }

    fn profile(name: &str) -> String {
        format!(
            r#"{{"response":{{"players":[{{"steamid":"{STEAMID}","personaname":"{name}","avatarmedium":"https://avatars.steamstatic.com/ab_medium.jpg","communityvisibilitystate":3}}]}}}}"#
        )
    }

    fn picture() -> Result<Resp, HttpError> {
        Ok(Resp { status: 200, eresult: None, body: JPEG.to_vec() })
    }

    fn link(rig: &mut Rig, key: &str, who: &str) -> Vec<FromEngine> {
        rig.send(Request::AuthKey { key: Secret::new(key), account: who.to_owned() })
    }

    /// An engine linked by key to the account `STEAMID`, named Rig, with the sends so far forgotten.
    fn linked() -> Rig {
        let mut rig = Rig::ready();
        rig.http.on("GetPlayerSummaries", [fake::reply(200, &profile("Rig"))]);
        rig.http.on("avatars.steamstatic.com", [picture()]);
        link(&mut rig, KEY, STEAMID);
        rig.http.forget();
        rig
    }

    fn library(rig: &mut Rig) -> Vec<FromEngine> {
        rig.send(Request::Library { target: target(), cursor: None })
    }

    fn failure(events: &[FromEngine]) -> (AuthFail, &str) {
        match events {
            [FromEngine { event: Event::Auth { state: AuthState::Failed { reason, message } }, .. }] => (*reason, message),
            other => panic!("expected one failure, got {other:?}"),
        }
    }

    fn targets_of(events: &[FromEngine]) -> Vec<Target> {
        events.iter().find_map(|e| if let Event::Targets { targets } = &e.event { Some(targets.clone()) } else { None }).expect("a targets event")
    }

    fn script_store(rig: &Rig) {
        rig.http.on("IStoreService", [fake::ok(r#"{"response":{"version_hash":"1","tags":[{"tagid":19,"name":"Action"}]}}"#)]);
        rig.http.on("IStoreBrowseService", [fake::ok(r#"{"response":{"store_items":[{"id":10,"success":1,"visible":true,"tagids":[19]}]}}"#)]);
    }

    #[test]
    fn the_key_is_checked_with_steam_then_kept_for_the_engines_user_only() {
        let mut rig = Rig::ready();
        rig.http.on("GetPlayerSummaries", [fake::reply(200, &profile("Rig"))]);
        rig.http.on("avatars.steamstatic.com", [picture()]);
        let events = link(&mut rig, "0123 4567-89ab cdef 0123456789abcdef0123", STEAMID);
        assert_eq!(failure(&events).0, AuthFail::BadKey, "more than 32 characters");

        let events = link(&mut rig, &KEY.to_lowercase(), STEAMID);
        assert_eq!(events[0], FromEngine::answer(3, Event::Auth { state: AuthState::Done { target: target() } }));
        let [target] = targets_of(&events).try_into().unwrap();
        assert_eq!((target.name.as_str(), target.detail.as_deref(), target.link), ("Rig", Some("Web API key"), Link::Linked));
        assert!(target.avatar.is_some());
        let url = format!("https://api.steampowered.com/ISteamUser/GetPlayerSummaries/v2/?key={KEY}&steamids={STEAMID}&format=json");
        assert_eq!(rig.http.to("GetPlayerSummaries").iter().map(|sent| sent.url.as_str()).collect::<Vec<_>>(), [url]);

        let path = rig.data().join("apikey.json");
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved, serde_json::json!({ "key": KEY, "steamid": STEAMID, "persona": "Rig" }));
        assert_eq!((fs::metadata(&path).unwrap().permissions().mode() & 0o777, fs::metadata(rig.data()).unwrap().permissions().mode() & 0o777), (0o600, 0o700));

        rig.restart();
        rig.hello();
        assert_eq!(rig.log.lines().last().map(String::as_str), Some("ready, linked by key"));
        let [target] = targets_of(&rig.send(Request::Targets)).try_into().unwrap();
        assert_eq!((target.name.as_str(), target.link), ("Rig", Link::Linked));
    }

    #[test]
    fn a_profile_address_or_name_is_turned_into_the_id_by_steam() {
        let mut rig = Rig::ready();
        rig.http.on("ResolveVanityURL", [fake::reply(200, &format!(r#"{{"response":{{"steamid":"{STEAMID}","success":1}}}}"#))]);
        rig.http.on("GetPlayerSummaries", [fake::reply(200, &profile("Rig"))]);
        rig.http.on("avatars.steamstatic.com", [picture()]);
        let events = link(&mut rig, KEY, "https://steamcommunity.com/id/rig_name/?l=english");
        assert!(matches!(events[0].event, Event::Auth { state: AuthState::Done { .. } }), "{events:?}");
        assert!(rig.http.to("ResolveVanityURL")[0].url.ends_with(&format!("?key={KEY}&vanityurl=rig_name&format=json")));

        let mut rig = Rig::ready();
        rig.http.on("ResolveVanityURL", [fake::reply(200, r#"{"response":{"success":42,"message":"No match"}}"#)]);
        assert_eq!(failure(&link(&mut rig, KEY, "nobody")), (AuthFail::BadKey, "No Steam profile has that name."));
        assert!(!rig.data().join("apikey.json").exists());
    }

    #[test]
    fn a_key_or_account_that_is_not_one_is_refused_before_steam_is_asked() {
        let mut rig = Rig::ready();
        for (key, who) in [("0123", STEAMID), ("G123456789ABCDEF0123456789ABCDEF", STEAMID), (KEY, "not a profile!"), (KEY, "12345"), (KEY, "https://example.com/profiles/76561198000000001")] {
            assert_eq!(failure(&link(&mut rig, key, who)).0, AuthFail::BadKey, "{key} {who}");
        }
        assert!(rig.http.sent().is_empty() && !rig.data().join("apikey.json").exists());
    }

    #[test]
    fn a_key_steam_turns_away_is_not_kept() {
        let mut rig = Rig::ready();
        rig.http.on("GetPlayerSummaries", [fake::reply(403, FORBIDDEN)]);
        let events = link(&mut rig, KEY, STEAMID);
        assert_eq!(failure(&events), (AuthFail::BadKey, "Steam did not accept this key. Check that all 32 characters were typed."));
        assert!(!rig.data().join("apikey.json").exists());
        assert_eq!(rig.http.count("GetPlayerSummaries"), 1, "a refusal is not tried again");

        rig.http.on("GetPlayerSummaries", [fake::reply(200, r#"{"response":{"players":[]}}"#)]);
        assert_eq!(failure(&link(&mut rig, KEY, STEAMID)), (AuthFail::BadKey, "Steam has no profile with that ID."));
        rig.http.on("GetPlayerSummaries", [fake::down()]);
        assert_eq!(failure(&link(&mut rig, KEY, STEAMID)).0, AuthFail::Network);
    }

    #[test]
    fn the_library_and_recent_play_are_read_with_the_key_and_the_sign_in_is_never_asked() {
        let mut rig = Rig::signed_in();
        rig.http.on("GetPlayerSummaries", [fake::reply(200, &profile("Rig"))]);
        rig.http.on("avatars.steamstatic.com", [picture()]);
        link(&mut rig, KEY, STEAMID);
        rig.http.forget();
        rig.http.on("GetOwnedGames", [fake::reply(200, &fake::owned(&[(10, "Ten", 120, 1_700_000_100)]))]);
        rig.http.on("GetRecentlyPlayedGames", [fake::reply(200, r#"{"response":{"total_count":1,"games":[{"appid":10,"playtime_2weeks":30}]}}"#)]);
        script_store(&rig);

        let events = library(&mut rig);
        let [FromEngine { event: Event::Library { entries, next: None, .. }, .. }] = events.as_slice() else { panic!("{events:?}") };
        let activity = &entries[0].activity;
        assert_eq!((entries[0].title.as_str(), activity.playtime_s, activity.recent_s, activity.last_played), ("Ten", Some(7200), Some(1800), Some(1_700_000_100)));
        let owned = &rig.http.to("GetOwnedGames")[0].url;
        assert!(owned.contains(&format!("key={KEY}&steamid={STEAMID}&include_appinfo=1")) && !owned.contains("access_token"), "{owned}");
        assert_eq!(rig.http.count("GenerateAccessTokenForApp"), 0);
        assert!(rig.data().join("token.json").exists(), "the sign-in is left as it was");
    }

    #[test]
    fn private_game_details_are_explained() {
        let mut rig = linked();
        rig.http.on("GetOwnedGames", [fake::reply(200, r#"{"response":{}}"#)]);
        let events = library(&mut rig);
        let [FromEngine { event: Event::Error { code, message }, .. }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!((*code, message.as_str()), (ErrorCode::BadRequest, "Steam returned no games. In Steam, set Game details to Public under Privacy Settings."));

        rig.http.on("GetOwnedGames", [fake::reply(200, r#"{"response":{"game_count":0}}"#)]);
        rig.http.on("GetRecentlyPlayedGames", [fake::reply(200, r#"{"response":{}}"#)]);
        script_store(&rig);
        let events = library(&mut rig);
        assert!(matches!(events.as_slice(), [FromEngine { event: Event::Library { entries, .. }, .. }] if entries.is_empty()), "{events:?}");
    }

    #[test]
    fn a_key_steam_withdraws_is_shown_as_needing_a_new_key_and_is_not_asked_again() {
        let mut rig = linked();
        rig.http.on("GetOwnedGames", [fake::reply(403, FORBIDDEN)]);
        let events = library(&mut rig);
        assert!(matches!(events[0].event, Event::Error { code: ErrorCode::SignIn, .. }), "{events:?}");
        let [target] = targets_of(&events).try_into().unwrap();
        assert_eq!((target.link, target.detail.as_deref()), (Link::Expired, Some("Web API key")));

        let before = rig.http.sent().len();
        assert!(matches!(library(&mut rig)[0].event, Event::Error { code: ErrorCode::SignIn, .. }));
        assert_eq!(rig.http.sent().len(), before, "nothing more is sent with a key Steam refused");
        // A new key mends it.
        rig.http.on("GetPlayerSummaries", [fake::reply(200, &profile("Rig"))]);
        link(&mut rig, KEY, STEAMID);
        assert_eq!(targets_of(&rig.send(Request::Targets))[0].link, Link::Linked);
    }

    #[test]
    fn a_request_for_a_pause_is_obeyed() {
        let mut rig = linked();
        rig.http.on("GetOwnedGames", [fake::reply(429, "")]);
        assert!(matches!(library(&mut rig)[0].event, Event::Error { code: ErrorCode::Network, .. }));
        library(&mut rig);
        assert_eq!(rig.http.count("GetOwnedGames"), 1);
        rig.clock.advance_s(PAUSE_S);
        library(&mut rig);
        assert_eq!(rig.http.count("GetOwnedGames"), 2);
        assert_eq!(targets_of(&rig.send(Request::Targets))[0].link, Link::Linked, "a pause is not a refusal");
    }

    #[test]
    fn a_key_for_another_account_replaces_the_sign_in_and_forget_removes_both() {
        let mut rig = Rig::signed_in();
        rig.http.on("GetPlayerSummaries", [fake::reply(200, &profile("Other").replace(STEAMID, OTHER))]);
        rig.http.on("avatars.steamstatic.com", [picture()]);
        let events = link(&mut rig, KEY, OTHER);
        assert!(matches!(events[0].event, Event::Auth { state: AuthState::Done { .. } }), "{events:?}");
        assert!(!rig.data().join("token.json").exists() && rig.data().join("apikey.json").exists());

        let mut rig = Rig::signed_in();
        rig.http.on("GetPlayerSummaries", [fake::reply(200, &profile("Rig"))]);
        rig.http.on("avatars.steamstatic.com", [picture()]);
        link(&mut rig, KEY, STEAMID);
        assert!(rig.data().join("token.json").exists() && rig.data().join("apikey.json").exists(), "the same account keeps both");
        let events = rig.send(Request::Forget { target: target() });
        assert_eq!(events, [FromEngine::answer(rig.seq, Event::Targets { targets: Vec::new() })]);
        assert!(!rig.data().join("token.json").exists() && !rig.data().join("apikey.json").exists());
    }

    #[test]
    fn the_owners_achievements_come_from_the_key_and_a_private_profile_is_no_progress() {
        let mut rig = linked();
        let game = lumagame_core::ids::ProviderGameId::new("10").unwrap();
        rig.http.on("GetPlayerAchievements", [fake::reply(200, r#"{"playerstats":{"achievements":[{"apiname":"A","achieved":1,"unlocktime":5},{"apiname":"B","achieved":0}]}}"#)]);
        let events = rig.send(Request::Achievements { target: target(), game: game.clone() });
        let [FromEngine { event: Event::Achievements { unlocked, .. }, .. }] = events.as_slice() else { panic!("{events:?}") };
        assert_eq!(unlocked.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["A"]);
        assert!(rig.http.to("GetPlayerAchievements")[0].url.contains(&format!("key={KEY}&steamid={STEAMID}&appid=10")));

        rig.http.on("GetPlayerAchievements", [fake::reply(403, PRIVATE)]);
        let events = rig.send(Request::Achievements { target: target(), game });
        assert!(matches!(events.as_slice(), [FromEngine { event: Event::Achievements { unlocked, .. }, .. }] if unlocked.is_empty()), "{events:?}");
        assert_eq!(targets_of(&rig.send(Request::Targets))[0].link, Link::Linked, "a private profile does not withdraw the key");
    }

    /// Everything the engine sent to the app or logged over a link, a library, an achievement read and a refusal.
    #[test]
    fn the_key_never_leaves_the_engine_in_an_event_a_log_or_a_debug_line() {
        let mut rig = Rig::ready();
        rig.http.on("GetPlayerSummaries", [fake::reply(200, &profile("Persona Name"))]);
        rig.http.on("avatars.steamstatic.com", [picture()]);
        rig.http.on("GetOwnedGames", [fake::reply(200, &fake::owned(&[(10, "Ten", 1, 1_700_000_000)]))]);
        rig.http.on("GetRecentlyPlayedGames", [fake::reply(200, r#"{"response":{}}"#)]);
        rig.http.on("GetPlayerAchievements", [fake::reply(403, FORBIDDEN)]);
        script_store(&rig);
        let request = Request::AuthKey { key: Secret::new(KEY), account: STEAMID.to_owned() };
        assert!(!format!("{request:?}").contains(KEY));
        let mut events = rig.send(request);
        events.extend(library(&mut rig));
        let game = lumagame_core::ids::ProviderGameId::new("10").unwrap();
        events.extend(rig.send(Request::Achievements { target: target(), game }));
        events.extend(link(&mut rig, "0123", STEAMID));
        let sent = events.iter().map(ipc::encode).collect::<Vec<_>>().join("\n");
        let logged = rig.log.lines().join("\n");
        for secret in [KEY, "key=", "api.steampowered.com", "GetOwnedGames"] {
            assert!(!sent.contains(secret) && !logged.contains(secret), "{secret} leaked");
        }
        for private in ["Persona Name", STEAMID] {
            assert!(!logged.contains(private), "{private} was logged: {logged}");
        }
        assert!(sent.contains("Persona Name"), "the app shows the profile name");
    }

    #[test]
    fn a_saved_key_that_is_not_one_is_left_alone_and_the_engine_starts_signed_out() {
        for text in [r#"{"key":"short","steamid":"76561198000000001"}"#, r#"{"key":"0123456789ABCDEF0123456789ABCDEF","steamid":"nope"}"#, "not json"] {
            let mut rig = Rig::new();
            files::write(&rig.data(), KEY_FILE, text.as_bytes()).unwrap();
            rig.hello();
            assert_eq!(rig.send(Request::Targets), [FromEngine::answer(2, Event::Targets { targets: Vec::new() })], "{text}");
            assert!(rig.data().join(KEY_FILE).exists());
        }
    }

    #[test]
    fn accounts_are_named_by_id_profile_address_or_name() {
        let id = |text: &str| parse_who(text);
        let vanity = |name: &str| Some(Who::Vanity(name.to_owned()));
        assert_eq!(id(STEAMID), Some(Who::Id(STEAMID.to_owned())));
        assert_eq!(id(" https://steamcommunity.com/profiles/76561198000000001/ "), Some(Who::Id(STEAMID.to_owned())));
        assert_eq!(id("https://www.steamcommunity.com/id/some-name"), vanity("some-name"));
        assert_eq!(id("some_name"), vanity("some_name"));
        for bad in ["", "1", "76561198000000", "99999999999999999", "a b", "https://steamcommunity.com/groups/x", "https://evil.example/id/x", "https://steamcommunity.com/id/"] {
            assert_eq!(id(bad), None, "{bad:?}");
        }
        assert_eq!(normalise_key(" 0123-4567 89ab cdef 0123 4567 89ab cdef "), Some(KEY.to_owned()));
    }
}
