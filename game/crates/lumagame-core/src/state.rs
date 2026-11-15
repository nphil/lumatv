//! What LumaGame keeps on the TV, and how an older file is brought forward.
//!
//! Three files, split by what losing each would cost:
//!
//! | File | Holds | If it is lost |
//! |---|---|---|
//! | `state.json` | [`State`]: each profile's favourites, hidden games, collections, remembered targets and history; which engines are on and their settings; the game identities | the owner's choices; so it is replaced atomically and the copy before is kept |
//! | `session.json` | [`OpenSession`]: the running session's playtime so far | at most a minute of playtime |
//! | `catalog.json` | [`crate::catalog::Listing`]: what the engines last listed | nothing; it is fetched again |
//!
//! An engine's credentials are in none of them. Each engine keeps its own under `engines/<id>/` and the app never reads them.
//!
//! `schema` is one number for the whole file. A field added with a default needs no new number: an older file simply lacks it.
//! Anything else (a rename, a moved value, a changed meaning) takes a new number and one [`Step`] that turns the shape before into
//! the new one, and a file is brought forward a step at a time. A file from a newer build is refused and left as it is, so going
//! back to an older build never destroys what the newer one wrote.

use crate::catalog::{Identity, Merge};
use crate::history::{History, Played};
use crate::ids::{EngineId, GameId, ProfileId, SourceKey};
use crate::launch::Remembered;
use crate::settings::Values;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The schema this build writes.
pub const SCHEMA: u32 = 1;

/// Brings a file's JSON from one schema to the next. `STEPS[n]` takes schema `n + 1` to `n + 2`.
pub type Step = fn(&mut serde_json::Value) -> Result<(), String>;

/// The steps of `state.json`. Empty while schema 1 is the only one there has been.
pub const STEPS: &[Step] = &[];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Not JSON, not this file's shape, or a step failed. The caller keeps the file aside and starts from the copy before, or new.
    Corrupt(String),
    /// Written by a newer build. The caller must not write over it.
    Newer { found: u32, supported: u32 },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Corrupt(why) => write!(f, "unreadable: {why}"),
            Self::Newer { found, supported } => write!(f, "schema {found} is newer than this build's {supported}"),
        }
    }
}

impl std::error::Error for LoadError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded<T> {
    pub value: T,
    /// The schema the file had, when it was older. The caller saves the migrated value, after keeping the old file aside.
    pub migrated_from: Option<u32>,
}

/// Reads a versioned file: checks its `schema`, applies the steps it still needs, then reads the result as `T`.
pub fn load_versioned<T: DeserializeOwned>(text: &str, current: u32, steps: &[Step]) -> Result<Loaded<T>, LoadError> {
    let corrupt = |why: String| LoadError::Corrupt(why);
    let mut json: serde_json::Value = serde_json::from_str(text).map_err(|e| corrupt(e.to_string()))?;
    let found = json.get("schema").and_then(serde_json::Value::as_u64).and_then(|n| u32::try_from(n).ok()).filter(|n| *n >= 1);
    let found = found.ok_or_else(|| corrupt("no schema number".into()))?;
    if found > current {
        return Err(LoadError::Newer { found, supported: current });
    }
    for from in found..current {
        let step = steps.get(from as usize - 1).ok_or_else(|| corrupt(format!("no step from schema {from}")))?;
        step(&mut json).map_err(|why| corrupt(format!("from schema {from}: {why}")))?;
        let object = json.as_object_mut().ok_or_else(|| corrupt(format!("from schema {from}: not an object")))?;
        object.insert("schema".into(), (from + 1).into());
    }
    let value = serde_json::from_value(json).map_err(|e| corrupt(e.to_string()))?;
    Ok(Loaded { value, migrated_from: (found != current).then_some(found) })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub schema: u32,
    /// Always a key of `profiles`: `load` refuses a file where it is not, and a profile is removed only after another is made active.
    pub active: ProfileId,
    pub profiles: BTreeMap<ProfileId, Profile>,
    /// Every engine the owner has touched. One that is not here is off, with its defaults.
    #[serde(default)]
    pub engines: BTreeMap<EngineId, EngineState>,
    /// Shared by the profiles: what counts as one game does not depend on who is playing.
    #[serde(default)]
    pub identity: Identity,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub user: UserData,
    #[serde(default)]
    pub history: History,
    /// The app's own view state (sort orders, the stats line), kept here so it follows the profile. The core never reads it.
    #[serde(default)]
    pub ui: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineState {
    /// An engine is off until the owner switches it on in Integrations. While off it is not started and nothing of it is shown.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub settings: Values,
}

/// One profile's choices.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserData {
    #[serde(default)]
    pub favourites: BTreeSet<GameId>,
    #[serde(default)]
    pub hidden: BTreeSet<GameId>,
    #[serde(default)]
    pub collections: Vec<Collection>,
    /// "Remember for this game".
    #[serde(default)]
    pub pinned: BTreeMap<GameId, SourceKey>,
    /// The target each game last ran on.
    #[serde(default)]
    pub last_ok: BTreeMap<GameId, SourceKey>,
    #[serde(default)]
    pub preferred_engine: Option<EngineId>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    pub name: String,
    pub games: Vec<GameId>,
}

impl UserData {
    pub fn remembered(&self, game: GameId) -> Remembered<'_> {
        Remembered { pinned: self.pinned.get(&game), preferred_engine: self.preferred_engine.as_ref(), last_ok: self.last_ok.get(&game) }
    }

    /// A session of `game` reached Running on `source`.
    pub fn started(&mut self, game: GameId, source: &SourceKey, pin: bool) {
        self.last_ok.insert(game, source.clone());
        if pin {
            self.pinned.insert(game, source.clone());
        }
    }

    /// Two games turned out to be one. Where both had a choice, the older game's stands.
    pub fn merge(&mut self, merge: Merge) {
        let Merge { from, into } = merge;
        for set in [&mut self.favourites, &mut self.hidden] {
            if set.remove(&from) {
                set.insert(into);
            }
        }
        for map in [&mut self.pinned, &mut self.last_ok] {
            if let Some(source) = map.remove(&from) {
                map.entry(into).or_insert(source);
            }
        }
        for collection in &mut self.collections {
            if collection.games.contains(&into) {
                collection.games.retain(|g| *g != from);
            } else {
                collection.games.iter_mut().filter(|g| **g == from).for_each(|g| *g = into);
            }
        }
    }
}

impl Default for State {
    /// The first start: one profile, every engine off.
    fn default() -> Self {
        let main = ProfileId::new("main").expect("a valid id");
        let profile = Profile { name: "Player".into(), ..Profile::default() };
        State { schema: SCHEMA, active: main.clone(), profiles: BTreeMap::from([(main, profile)]), engines: BTreeMap::new(), identity: Identity::default() }
    }
}

impl State {
    pub fn load(text: &str) -> Result<Loaded<State>, LoadError> {
        let loaded: Loaded<State> = load_versioned(text, SCHEMA, STEPS)?;
        if !loaded.value.profiles.contains_key(&loaded.value.active) {
            return Err(LoadError::Corrupt("the active profile does not exist".into()));
        }
        Ok(loaded)
    }

    pub fn save(&self) -> String {
        // Only strings, numbers and maps keyed by strings: nothing here can fail to serialise.
        serde_json::to_string(self).expect("the state serialises")
    }

    pub fn profile(&self) -> &Profile {
        &self.profiles[&self.active]
    }

    pub fn profile_mut(&mut self) -> &mut Profile {
        self.profiles.get_mut(&self.active).expect("the active profile exists")
    }

    pub fn enabled_engines(&self) -> BTreeSet<EngineId> {
        self.engines.iter().filter(|(_, e)| e.enabled).map(|(id, _)| id.clone()).collect()
    }

    /// Applies what a catalog build found to everything kept by game, in every profile.
    pub fn apply_merges(&mut self, merges: &[Merge]) {
        for merge in merges {
            for profile in self.profiles.values_mut() {
                profile.user.merge(*merge);
                profile.history.merge(*merge);
            }
        }
    }

    /// Adds a finished or recovered session to its profile's history. False when the profile is gone or the session is already there.
    pub fn record(&mut self, session: OpenSession) -> bool {
        self.profiles.get_mut(&session.profile).is_some_and(|profile| profile.history.record(session.played))
    }
}

/// `session.json`: the session on screen, rewritten at each checkpoint and removed when the session is recorded. Found at a start,
/// it is a session the app did not live to record: a crash, a power cut. It has no schema number: a copy that cannot be read is
/// dropped, at the cost of that one session's time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenSession {
    pub profile: ProfileId,
    pub played: Played,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> SourceKey {
        s.parse().unwrap()
    }

    #[test]
    fn a_first_start_has_one_profile_and_every_engine_off_and_reads_back_as_written() {
        let mut state = State::default();
        assert_eq!((state.schema, state.profile().name.as_str(), state.enabled_engines().len()), (SCHEMA, "Player", 0));

        let gfn = EngineId::new("gfn").unwrap();
        state.engines.insert(gfn.clone(), EngineState { enabled: true, settings: Values::from([("hdr".into(), crate::settings::Value::Bool(true))]) });
        state.engines.insert(EngineId::new("punktfunk").unwrap(), EngineState::default());
        state.profile_mut().user.favourites.insert(GameId(3));
        state.profile_mut().user.started(GameId(3), &key("gfn|acct|100"), true);
        state.profile_mut().ui = serde_json::json!({"stats": true});
        assert_eq!(state.enabled_engines(), BTreeSet::from([gfn]));

        let loaded = State::load(&state.save()).unwrap();
        assert_eq!(loaded, Loaded { value: state, migrated_from: None });
    }

    #[test]
    fn a_file_that_lacks_a_later_addition_loads_with_its_default() {
        let loaded = State::load(r#"{"schema":1,"active":"main","profiles":{"main":{"name":"Nitin"}},"added_later":{"x":1}}"#).unwrap().value;
        assert_eq!(loaded.profile().user, UserData::default());
        assert!(loaded.engines.is_empty() && loaded.profile().ui.is_null());
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct Doc {
        schema: u32,
        title: String,
        minutes: u32,
    }

    fn rename_name_to_title(json: &mut serde_json::Value) -> Result<(), String> {
        let name = json.as_object_mut().and_then(|o| o.remove("name")).ok_or("no name")?;
        json["title"] = name;
        Ok(())
    }

    fn seconds_to_minutes(json: &mut serde_json::Value) -> Result<(), String> {
        let seconds = json.get("seconds").and_then(serde_json::Value::as_u64).ok_or("no seconds")?;
        json["minutes"] = (seconds / 60).into();
        Ok(())
    }

    const DOC_STEPS: &[Step] = &[rename_name_to_title, seconds_to_minutes];

    #[test]
    fn an_older_file_is_brought_forward_one_step_at_a_time() {
        let current = Doc { schema: 3, title: "Hades".into(), minutes: 2 };
        let from_1 = load_versioned::<Doc>(r#"{"schema":1,"name":"Hades","seconds":120}"#, 3, DOC_STEPS).unwrap();
        assert_eq!(from_1, Loaded { value: Doc { schema: 3, title: "Hades".into(), minutes: 2 }, migrated_from: Some(1) });
        let from_2 = load_versioned::<Doc>(r#"{"schema":2,"title":"Hades","seconds":120}"#, 3, DOC_STEPS).unwrap();
        assert_eq!(from_2, Loaded { value: current, migrated_from: Some(2) });
        let at_3 = load_versioned::<Doc>(r#"{"schema":3,"title":"Hades","minutes":2}"#, 3, DOC_STEPS).unwrap();
        assert_eq!(at_3.migrated_from, None);
    }

    #[test]
    fn a_newer_file_is_refused_and_a_broken_one_is_called_corrupt() {
        let load = |text: &str| load_versioned::<Doc>(text, 3, DOC_STEPS).unwrap_err();
        assert_eq!(load(r#"{"schema":4,"title":"Hades","minutes":2,"extra":true}"#), LoadError::Newer { found: 4, supported: 3 });
        assert_eq!(State::load(r#"{"schema":2}"#).unwrap_err(), LoadError::Newer { found: 2, supported: SCHEMA });
        for broken in ["", "{", "[]", r#"{"title":"Hades"}"#, r#"{"schema":0}"#, r#"{"schema":"1"}"#, r#"{"schema":3,"title":"Hades"}"#] {
            assert!(matches!(load(broken), LoadError::Corrupt(_)), "{broken:?}");
        }
        assert_eq!(load(r#"{"schema":1,"seconds":120}"#), LoadError::Corrupt("from schema 1: no name".into()));
        assert!(
            matches!(load_versioned::<Doc>(r#"{"schema":1,"name":"Hades","seconds":120}"#, 3, &DOC_STEPS[..1]).unwrap_err(), LoadError::Corrupt(_)),
            "a step is missing"
        );
        assert!(matches!(State::load(r#"{"schema":1,"active":"gone","profiles":{"main":{"name":"Nitin"}}}"#).unwrap_err(), LoadError::Corrupt(_)));
    }

    #[test]
    fn a_started_session_is_what_play_remembers() {
        let mut user = UserData { preferred_engine: Some(EngineId::new("gfn").unwrap()), ..UserData::default() };
        let (cloud, pc) = (key("gfn|acct|100"), key("punktfunk|pc1|steam:1"));
        user.started(GameId(1), &cloud, false);
        user.started(GameId(1), &pc, true);
        user.started(GameId(1), &cloud, false);
        let remembered = user.remembered(GameId(1));
        assert_eq!((remembered.pinned, remembered.last_ok, remembered.preferred_engine.map(EngineId::as_str)), (Some(&pc), Some(&cloud), Some("gfn")));
        let other = user.remembered(GameId(2));
        assert!(other.pinned.is_none() && other.last_ok.is_none() && other.preferred_engine.is_some());
    }

    #[test]
    fn merged_games_keep_one_set_of_choices_in_every_profile() {
        let mut state = State::default();
        let kid = ProfileId::new("kid").unwrap();
        state.profiles.insert(kid.clone(), Profile { name: "Kid".into(), ..Profile::default() });

        let (cloud, pc) = (key("gfn|acct|100"), key("punktfunk|pc1|steam:1"));
        let user = &mut state.profile_mut().user;
        user.favourites.insert(GameId(5));
        user.hidden.insert(GameId(2));
        user.started(GameId(2), &cloud, true);
        user.started(GameId(5), &pc, true);
        user.collections =
            vec![Collection { name: "Co-op".into(), games: vec![GameId(5), GameId(9)] }, Collection { name: "Both".into(), games: vec![GameId(2), GameId(5)] }];
        assert!(state.record(OpenSession { profile: kid.clone(), played: Played { game: GameId(5), source: pc.clone(), start: 1_000, seconds: 60 } }));
        assert!(!state.record(OpenSession {
            profile: ProfileId::new("gone").unwrap(),
            played: Played { game: GameId(5), source: pc.clone(), start: 2_000, seconds: 60 }
        }));

        state.apply_merges(&[Merge { from: GameId(5), into: GameId(2) }]);
        let user = &state.profile().user;
        assert_eq!(user.favourites, BTreeSet::from([GameId(2)]));
        assert_eq!(user.hidden, BTreeSet::from([GameId(2)]));
        assert_eq!(user.pinned, BTreeMap::from([(GameId(2), cloud.clone())]), "the older game's choice stands");
        assert_eq!(user.last_ok, BTreeMap::from([(GameId(2), cloud)]));
        assert_eq!(user.collections[0].games, [GameId(2), GameId(9)]);
        assert_eq!(user.collections[1].games, [GameId(2)]);
        assert_eq!(state.profiles[&kid].history.total(GameId(2)).seconds, 60);
        assert_eq!(state.profiles[&kid].history.total(GameId(5)).seconds, 0);
    }
}
