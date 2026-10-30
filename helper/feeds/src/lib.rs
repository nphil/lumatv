//! Weather and news for LumaTV's Home.
//!
//! The helper serves Home's weather and news cards from a cache so they draw at once on a cold start. This crate is the
//! pure part: given a [`fetch::Fetch`] it fetches, normalises and returns documents, and it stores and loads the last good
//! document on disk. It has no threads, no clocks (every function takes `now_ms`, Unix epoch milliseconds), no network of
//! its own and no logging; the helper's runner decides when to call it.
#![forbid(unsafe_code)]

mod calendar;
mod error;
mod jsnum;

pub mod cache;
pub mod news;
pub mod weather;
pub mod weather_ha;
pub mod weather_more;

pub use error::FeedError;

/// The `schema` field of every stored document; a file with another value is ignored rather than misread.
pub const SCHEMA: u32 = 1;

/// 15 min: a reading older than this is stale and due again (the QML widgets' refresh interval).
pub const REFRESH_MS: u64 = 900_000;

/// 6 h: a reading older than this is never shown.
pub const EXPIRE_MS: u64 = 6 * 3_600_000;

/// Delay after the 1st, 2nd and 3rd-and-later consecutive failure of a refresh.
pub const RETRY_MS: [u64; 3] = [120_000, 600_000, 1_800_000];
