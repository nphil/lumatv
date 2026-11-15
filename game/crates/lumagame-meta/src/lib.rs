//! Artwork is Steam-first, source-second, optional-provider-last. No title can establish identity.
//! Network, disk and image work belongs on [`Loader`]'s bounded workers, never the drawing thread.

mod cache;
mod decode;
mod loader;
pub mod providers;
pub mod secrets;

pub use cache::{Cache, CacheLimits};
pub use decode::{Density, Image, ImageSpec};
pub use loader::{Done, Job, Loaded, Loader};
pub use providers::{Client, Http, Transport};
use lumagame_core::{catalog::Game, ids::ExternalId};
use serde::{Deserialize, Serialize};
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    Http(u16),
    Invalid(&'static str),
    Disabled,
    WorkerClosed,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "artwork storage: {e}"),
            Self::Json(_) => f.write_str("invalid metadata response"),
            Self::Http(code) => write!(f, "artwork provider HTTP {code}"),
            Self::Invalid(reason) => f.write_str(reason),
            Self::Disabled => f.write_str("provider credentials are not configured"),
            Self::WorkerClosed => f.write_str("artwork worker stopped"),
        }
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self { Self::Json(e) }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind { Cover, Hero, Logo, Header, Icon }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artwork {
    pub kind: Kind,
    pub url: String,
    pub credit: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub external: Vec<ExternalId>,
    pub title: String,
    pub summary: String,
    pub release_date: Option<i64>,
    pub genres: Vec<String>,
    pub developers: Vec<String>,
    pub publishers: Vec<String>,
    pub artwork: Vec<Artwork>,
    pub credit: String,
}

pub const STEAM_CREDIT: &str = "Game artwork provided by Steam. Steam is a trademark of Valve Corporation.";
pub const SGDB_CREDIT: &str = "Artwork provided by SteamGridDB and its contributors (steamgriddb.com).";
pub const IGDB_CREDIT: &str = "Game metadata and artwork provided by IGDB (igdb.com).";

/// Steam's uniform artwork leads; missing files fall through without changing the game's identity.
pub fn keyless_art(game: &Game, kind: Kind) -> Vec<Artwork> {
    let mut art = Vec::new();
    if let Some(id) = game.external_in(ExternalId::STEAM).and_then(numeric_id) {
        let files: &[&str] = match kind {
            Kind::Cover => &["library_600x900.jpg", "library_600x900_2x.jpg"],
            Kind::Hero => &["library_hero.jpg"],
            Kind::Logo => &["logo.png"],
            Kind::Header => &["header.jpg"],
            Kind::Icon => &[],
        };
        art.extend(files.iter().map(|file| Artwork {
            kind, url: format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/{file}"),
            credit: STEAM_CREDIT.into(),
        }));
    }
    for source in &game.sources {
        let own = &source.entry.art;
        let url = match kind {
            Kind::Cover => &own.cover,
            Kind::Hero | Kind::Header => &own.hero,
            Kind::Logo => &own.logo,
            Kind::Icon => &own.icon,
        };
        if let Some(url) = url.as_ref().filter(|url| !art.iter().any(|a| &a.url == *url)) {
            art.push(Artwork { kind, url: url.clone(), credit: format!("Artwork supplied by {}.", source.engine) });
        }
    }
    art
}

pub(crate) fn numeric_id(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) { return None; }
    value.parse::<u64>().ok().filter(|n| *n > 0)
}

/// CPU-only decoding for an engine/cache file, called from a bounded artwork worker.
pub fn decode_image(bytes: &[u8], spec: ImageSpec) -> Result<Image> {
    decode::normalize(decode::read(bytes)?, spec)
}
