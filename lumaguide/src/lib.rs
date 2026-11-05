//! An antenna TV guide from online listings, for a tuner whose broadcast guide (the stations' own EPG) is thin.
//!
//! The listings come from up to three sources, picked by zip code and tried in the owner's order: a Plex server's guide
//! ([`provider::plex`]), Schedules Direct ([`provider::sd`]) and the free Gracenote grid ([`provider::gracenote`]). Each source's
//! stations are matched to the broadcast channels ([`map`]: by virtual channel, then call sign, then RF channel), a week of
//! programmes is fetched and kept ([`guide`]), and the result is merged with the broadcast guide ([`merge`]: online wins where it has
//! something, the broadcast fills the gaps). The output is the guide data model the TV's guide and banner draw ([`Programme`]) and
//! XMLTV ([`xmltv`]).
//!
//! Nothing here opens a socket or a file: the host passes the network ([`net::Http`]) and the storage ([`store::Store`]), so the same
//! engine runs on the TV's helper and in an app of its own.

pub mod guide;
pub mod map;
pub mod merge;
pub mod model;
pub mod net;
pub mod provider;
pub mod store;
pub mod time;
pub mod xmltv;

pub use guide::{Guide, Listings, Settings, SourceSettings, SourceStatus};
pub use model::{Channel, Programme};
pub use net::{Http, Method, NetError, Request, Response};
pub use provider::Source;
pub use store::{DirStore, MemStore, Store};
