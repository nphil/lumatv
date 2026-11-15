//! Identifiers. Each is a type of its own, checked when it is made, so one kind cannot stand in for another and what an engine sends
//! is bounded before anything keeps it. The checked strings travel as plain JSON strings, and so can be the keys of a JSON object.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// A string that is not a valid id of the kind named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdError {
    pub kind: &'static str,
    pub value: String,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} is not a valid {}", self.value, self.kind)
    }
}

impl std::error::Error for IdError {}

/// Lowercase letters, digits and hyphens, starting with a letter: `gfn`, `steam`, `main`.
fn is_slug(s: &str, max: usize) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= max && b[0].is_ascii_lowercase() && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

fn is_text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}

macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident, $kind:literal, $valid:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                let valid: fn(&str) -> bool = $valid;
                if valid(&value) { Ok(Self(value)) } else { Err(IdError { kind: $kind, value }) }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = IdError;
            fn try_from(value: String) -> Result<Self, IdError> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// An engine: `gfn`, `punktfunk`, `gamestream`, `native`. Also the name of its directory in the package and in the data directory.
    EngineId,
    "engine id",
    |s| is_slug(s, 32)
);

string_id!(
    /// Where an engine plays: an account, a PC, this TV. The engine's own stable id for it, in printable ASCII without `|` (the
    /// separator of a [`SourceKey`]); an engine whose natural id is anything else sends it in hex.
    TargetId,
    "target id",
    |s| s.len() <= 128 && !s.is_empty() && s.bytes().all(|c| c.is_ascii_graphic() && c != b'|')
);

string_id!(
    /// An engine's own id for a game on one target, and the handle it launches by. The same across restarts and library refreshes:
    /// an engine whose source has no such id derives one that is.
    ProviderGameId,
    "provider game id",
    |s| is_text(s, 256)
);

string_id!(
    /// A person's profile on this TV.
    ProfileId,
    "profile id",
    |s| is_slug(s, 32)
);

/// LumaGame's own id for a game, given when the game is first seen and kept in the saved state ([`crate::catalog::Identity`]). A
/// lower number is an older game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GameId(pub u64);

/// One launch, numbered by the app. An engine echoes it on everything about that launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub u64);

/// A game's identity in a store or database that is the same on every PC and account: `steam:1245620`, `gfn:100123`,
/// `webos:com.example.game`. Two library entries that share one are the same game. An id that is only good on one host (a custom
/// entry, an app index) is not one of these.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExternalId {
    ns: String,
    id: String,
}

impl ExternalId {
    pub const STEAM: &'static str = "steam";
    pub const EPIC: &'static str = "epic";
    pub const GOG: &'static str = "gog";
    /// GeForce NOW's id for a game, above its per-store variants.
    pub const GFN: &'static str = "gfn";
    /// A webOS app id.
    pub const WEBOS: &'static str = "webos";
    pub const IGDB: &'static str = "igdb";

    pub fn new(ns: impl Into<String>, id: impl Into<String>) -> Result<Self, IdError> {
        let (ns, id) = (ns.into(), id.into());
        if is_slug(&ns, 16) && is_text(&id, 128) { Ok(Self { ns, id }) } else { Err(IdError { kind: "external id", value: format!("{ns}:{id}") }) }
    }

    pub fn ns(&self) -> &str {
        &self.ns
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

impl FromStr for ExternalId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, IdError> {
        let (ns, id) = s.split_once(':').ok_or_else(|| IdError { kind: "external id", value: s.to_owned() })?;
        Self::new(ns, id)
    }
}

impl TryFrom<String> for ExternalId {
    type Error = IdError;
    fn try_from(value: String) -> Result<Self, IdError> {
        value.parse()
    }
}

impl From<ExternalId> for String {
    fn from(id: ExternalId) -> String {
        id.to_string()
    }
}

impl fmt::Display for ExternalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.ns, self.id)
    }
}

/// One way to play one game: the engine, where it plays, and the engine's id for the game there. Written `engine|target|id`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SourceKey {
    pub engine: EngineId,
    pub target: TargetId,
    pub id: ProviderGameId,
}

impl FromStr for SourceKey {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, IdError> {
        // Neither an engine id nor a target id can hold a bar, so the first two always split it; the game's id keeps any of its own.
        let mut parts = s.splitn(3, '|');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(engine), Some(target), Some(id)) => Ok(Self { engine: EngineId::new(engine)?, target: TargetId::new(target)?, id: ProviderGameId::new(id)? }),
            _ => Err(IdError { kind: "source key", value: s.to_owned() }),
        }
    }
}

impl TryFrom<String> for SourceKey {
    type Error = IdError;
    fn try_from(value: String) -> Result<Self, IdError> {
        value.parse()
    }
}

impl From<SourceKey> for String {
    fn from(key: SourceKey) -> String {
        key.to_string()
    }
}

impl fmt::Display for SourceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}|{}|{}", self.engine, self.target, self.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn each_kind_of_id_takes_only_its_own_shape() {
        assert!(EngineId::new("gfn").is_ok() && EngineId::new("steam-link2").is_ok());
        for bad in ["", "GFN", "2k", "a b", "a|b", &"a".repeat(33)] {
            assert!(EngineId::new(bad).is_err(), "{bad:?}");
        }
        assert!(TargetId::new("AA:BB@host.local").is_ok());
        for bad in ["", "a|b", "a b", "h\u{f6}st", &"a".repeat(129)] {
            assert!(TargetId::new(bad).is_err(), "{bad:?}");
        }
        assert!(ProviderGameId::new("steam:1245620").is_ok() && ProviderGameId::new("Half-Life 2 | Demo").is_ok());
        assert!(ProviderGameId::new("").is_err() && ProviderGameId::new("a\nb").is_err());
        assert_eq!(EngineId::new("Bad").unwrap_err().to_string(), "\"Bad\" is not a valid engine id");
    }

    #[test]
    fn an_external_id_is_a_namespace_and_an_id_and_the_id_keeps_its_colons() {
        let id: ExternalId = "epic:ns:catalog:item".parse().unwrap();
        assert_eq!((id.ns(), id.id()), ("epic", "ns:catalog:item"));
        assert_eq!(id.to_string(), "epic:ns:catalog:item");
        assert_eq!(ExternalId::new(ExternalId::STEAM, "1245620").unwrap(), "steam:1245620".parse().unwrap());
        for bad in ["steam", "steam:", ":1", "Steam:1"] {
            assert!(bad.parse::<ExternalId>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_source_key_round_trips_as_a_string_and_as_a_json_object_key() {
        let key: SourceKey = "punktfunk|a1b2c3|custom:my|game".parse().unwrap();
        assert_eq!((key.engine.as_str(), key.target.as_str(), key.id.as_str()), ("punktfunk", "a1b2c3", "custom:my|game"));
        assert_eq!(key.to_string().parse::<SourceKey>().unwrap(), key);
        assert!("gfn|acct".parse::<SourceKey>().is_err() && "gfn||1".parse::<SourceKey>().is_err());

        let map = BTreeMap::from([(key.clone(), GameId(7))]);
        let json = serde_json::to_string(&map).unwrap();
        assert_eq!(json, r#"{"punktfunk|a1b2c3|custom:my|game":7}"#);
        assert_eq!(serde_json::from_str::<BTreeMap<SourceKey, GameId>>(&json).unwrap(), map);
        assert!(serde_json::from_str::<BTreeMap<SourceKey, GameId>>(r#"{"GFN|a|1":7}"#).is_err(), "a bad key is refused when it is read");

        let by_game = BTreeMap::from([(GameId(7), key)]);
        let json = serde_json::to_string(&by_game).unwrap();
        assert_eq!(serde_json::from_str::<BTreeMap<GameId, SourceKey>>(&json).unwrap(), by_game);
    }
}
