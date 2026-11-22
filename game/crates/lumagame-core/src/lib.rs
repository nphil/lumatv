//! LumaGame's rules with no TV in them.
//!
//! One app plays games from several engines (GeForce NOW, Punktfunk, Sunshine/Apollo, the TV's own apps, later more). Everything that
//! decides what the app does, and nothing that touches a window, a socket, a file or LG's libraries, is here, so the same rules run
//! in the app, in every engine process and in host tests.
//!
//! | Module | Holds |
//! |---|---|
//! | [`ids`] | the identifiers, each a checked type of its own |
//! | [`engine`] | what an engine says of itself (manifest, capabilities) and the trait the app drives |
//! | [`ipc`] | the messages between the app and an engine in a process of its own, one JSON object a line |
//! | [`settings`] | an engine's settings page as data the app draws |
//! | [`catalog`] | one card per game: which library entries are the same game, never by title alone |
//! | [`history`] | what was played and for how long, counted only for sessions that started |
//! | [`activity`] | what an account knows about a game (last played, play time, favourite, genres) and how it joins this TV's history |
//! | [`launch`] | which target Play uses, and when it must ask |
//! | [`session`] | one launch from Checking to its end, on a clock the caller supplies |
//! | [`state`] | the saved state, its schema version and the steps that bring an older file forward |
//!
//! `docs/lumagame/ARCHITECTURE.md` has the decisions behind these.

/// An enum that travels as one kebab-case word. A word this build does not know reads as the `unknown` variant instead of failing the
/// whole message: the app and the engines are built apart (one engine lives in another repository), so each has to outlive a word
/// the other adds.
macro_rules! word_enum {
    (
        $(#[$meta:meta])*
        pub enum $name:ident { $($(#[$vmeta:meta])* $variant:ident => $word:literal,)+ }
        unknown => $(#[$ometa:meta])* $other:ident
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)+
            /// A word from a newer build.
            $(#[$ometa])*
            $other,
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $word,)+
                    Self::$other => "unknown",
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let word = <String as serde::Deserialize>::deserialize(deserializer)?;
                Ok(match word.as_str() {
                    $($word => Self::$variant,)+
                    _ => Self::$other,
                })
            }
        }
    };
}

pub mod activity;
pub mod catalog;
pub mod engine;
pub mod history;
pub mod ids;
pub mod ipc;
pub mod launch;
pub mod session;
pub mod settings;
pub mod state;
