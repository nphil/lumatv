use crate::{Artwork, Error, IGDB_CREDIT, Kind, Metadata, Result, SGDB_CREDIT, numeric_id, secrets::Secrets};
use lumagame_core::ids::ExternalId;
use serde::{Deserialize, de::DeserializeOwned};
use std::{collections::VecDeque, io::Read, sync::{Arc, Condvar, Mutex}, time::{Duration, Instant}};

const JSON_LIMIT: usize = 1 << 20;
pub const IMAGE_LIMIT: usize = 16 << 20;

/// Requests intentionally have no Debug implementation because their headers may carry keys.
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub limit: usize,
}
pub struct Response { pub status: u16, pub body: Vec<u8> }
pub trait Transport: Send + Sync + 'static { fn execute(&self, request: Request) -> Result<Response>; }

pub struct Http { agent: ureq::Agent }
impl Default for Http {
    fn default() -> Self {
        Self { agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(20)).redirects(0).user_agent("LumaGame/0.1").build() }
    }
}
impl Transport for Http {
    fn execute(&self, request: Request) -> Result<Response> {
        if !(request.url.starts_with("https://") || request.url.starts_with("http://")) {
            return Err(Error::Invalid("unsupported artwork URL scheme"));
        }
        let mut outgoing = self.agent.request(request.method, &request.url);
        for (name, value) in &request.headers { outgoing = outgoing.set(name, value); }
        let response = if request.method == "GET" { outgoing.call() } else { outgoing.send_bytes(&request.body) };
        let response = match response {
            Ok(response) | Err(ureq::Error::Status(_, response)) => response,
            Err(_) => return Err(Error::Invalid("artwork provider connection failed")),
        };
        let status = response.status();
        let mut body = Vec::new();
        response.into_reader().take(request.limit as u64 + 1).read_to_end(&mut body)?;
        if body.len() > request.limit { return Err(Error::Invalid("artwork response exceeds byte limit")); }
        Ok(Response { status, body })
    }
}

struct Token { value: String, expires: Instant }
/// A clone shares the token and IGDB's rate/concurrency budget with the original client.
pub struct Client<T: Transport = Http> {
    transport: Arc<T>,
    secrets: Arc<Secrets>,
    token: Arc<Mutex<Option<Token>>>,
    gate: Arc<Gate>,
}
impl<T: Transport> Clone for Client<T> {
    fn clone(&self) -> Self {
        Self { transport: self.transport.clone(), secrets: self.secrets.clone(), token: self.token.clone(), gate: self.gate.clone() }
    }
}
impl Client<Http> {
    pub fn new(secrets: Secrets) -> Self { Self::with_transport(Http::default(), secrets) }
}
impl<T: Transport> Client<T> {
    pub fn with_transport(transport: T, secrets: Secrets) -> Self {
        Self { transport: Arc::new(transport), secrets: Arc::new(secrets), token: Arc::new(Mutex::new(None)), gate: Arc::new(Gate::default()) }
    }

    pub fn image(&self, url: &str) -> Result<Vec<u8>> {
        checked(self.transport.execute(Request { method: "GET", url: url.into(), headers: vec![], body: vec![], limit: IMAGE_LIMIT })?)
    }

    /// Only a shared Steam app id is accepted; SGDB's fuzzy title search is never used.
    pub fn steamgriddb(&self, external: &[ExternalId], kind: Kind) -> Result<Vec<Artwork>> {
        if self.secrets.steamgriddb_key.is_empty() { return Err(Error::Disabled); }
        let Some(id) = external.iter().find(|id| id.ns() == ExternalId::STEAM).and_then(|id| numeric_id(id.id())) else { return Ok(vec![]) };
        let (endpoint, query) = match kind {
            Kind::Cover => ("grids", "dimensions=600x900&types=static"),
            Kind::Hero => ("heroes", "types=static"),
            Kind::Logo => ("logos", "types=static"),
            Kind::Header => ("grids", "dimensions=460x215,920x430&types=static"),
            Kind::Icon => ("icons", "types=static"),
        };
        let bytes = checked(self.transport.execute(Request {
            method: "GET", url: format!("https://www.steamgriddb.com/api/v2/{endpoint}/steam/{id}?{query}"),
            headers: vec![("Authorization".into(), format!("Bearer {}", self.secrets.steamgriddb_key))], body: vec![], limit: JSON_LIMIT,
        })?)?;
        let response: SgResponse = serde_json::from_slice(&bytes)?;
        if !response.success { return Err(Error::Invalid("SteamGridDB rejected the request")); }
        Ok(response.data.into_iter().filter(|a| safe_remote(&a.url)).map(|a| Artwork {
            kind, url: a.url, credit: a.author.map(|author| format!("Artwork by {} on SteamGridDB (steamgriddb.com).", author.name)).unwrap_or_else(|| SGDB_CREDIT.into()),
        }).collect())
    }

    /// Steam is resolved through external_games, not a name search; an IGDB id is queried directly.
    pub fn igdb(&self, external: &[ExternalId]) -> Result<Option<Metadata>> {
        if self.secrets.twitch_client_id.is_empty() || self.secrets.twitch_client_secret.is_empty() { return Err(Error::Disabled); }
        let igdb = external.iter().find(|id| id.ns() == ExternalId::IGDB).and_then(|id| numeric_id(id.id()));
        let steam = external.iter().find(|id| id.ns() == ExternalId::STEAM).and_then(|id| numeric_id(id.id()));
        if let Some(id) = igdb {
            let query = format!("fields {GAME_FIELDS}; where id = {id}; limit 1;");
            let games: Vec<IgGame> = self.query("games", query)?;
            Ok(games.into_iter().find(|game| game.id == id).map(IgGame::metadata))
        } else if let Some(id) = steam {
            let fields = GAME_FIELDS.split(',').map(|field| format!("game.{field}")).collect::<Vec<_>>().join(",");
            let query = format!("fields uid,external_game_source,{fields}; where external_game_source = 1 & uid = \"{id}\"; limit 10;");
            let games: Vec<ExternalGame> = self.query("external_games", query)?;
            let mut matched = games.into_iter().filter(|game| game.external_game_source == 1 && numeric_id(&game.uid) == Some(id)).map(|game| game.game);
            let Some(game) = matched.next() else { return Ok(None) };
            // Ambiguous store records cannot choose a game on the owner's behalf.
            if matched.any(|other| other.id != game.id) { return Ok(None); }
            let mut metadata = game.metadata();
            metadata.external.push(ExternalId::new(ExternalId::STEAM, id.to_string()).expect("numeric Steam id"));
            Ok(Some(metadata))
        } else { Ok(None) }
    }

    fn query<R: DeserializeOwned>(&self, endpoint: &str, query: String) -> Result<R> {
        let token = self.access_token()?;
        let _permit = self.gate.enter()?;
        let response = self.transport.execute(Request {
            method: "POST", url: format!("https://api.igdb.com/v4/{endpoint}"),
            headers: vec![("Client-ID".into(), self.secrets.twitch_client_id.clone()), ("Authorization".into(), format!("Bearer {token}")), ("Content-Type".into(), "text/plain".into())],
            body: query.into_bytes(), limit: JSON_LIMIT,
        })?;
        drop(_permit);
        if response.status == 401 {
            *self.token.lock().map_err(|_| Error::WorkerClosed)? = None;
        }
        Ok(serde_json::from_slice(&checked(response)?)?)
    }

    fn access_token(&self) -> Result<String> {
        let mut token = self.token.lock().map_err(|_| Error::WorkerClosed)?;
        if let Some(token) = token.as_ref().filter(|t| t.expires > Instant::now()) { return Ok(token.value.clone()); }
        let _permit = self.gate.enter()?;
        let body = format!("client_id={}&client_secret={}&grant_type=client_credentials", encode(&self.secrets.twitch_client_id), encode(&self.secrets.twitch_client_secret));
        let response = self.transport.execute(Request {
            method: "POST", url: "https://id.twitch.tv/oauth2/token".into(),
            headers: vec![("Content-Type".into(), "application/x-www-form-urlencoded".into())], body: body.into_bytes(), limit: JSON_LIMIT,
        })?;
        let auth: Auth = serde_json::from_slice(&checked(response)?)?;
        if auth.access_token.is_empty() || !auth.token_type.eq_ignore_ascii_case("bearer") || auth.expires_in == 0 {
            return Err(Error::Invalid("invalid Twitch access token response"));
        }
        drop(_permit);
        let expires = Instant::now().checked_add(Duration::from_secs(auth.expires_in.saturating_sub(30))).ok_or(Error::Invalid("invalid Twitch token lifetime"))?;
        let value = auth.access_token;
        *token = Some(Token { value: value.clone(), expires });
        Ok(value)
    }
}
fn checked(response: Response) -> Result<Vec<u8>> {
    if !(200..300).contains(&response.status) { return Err(Error::Http(response.status)); }
    Ok(response.body)
}
fn encode(value: &str) -> String {
    let mut encoded = String::new();
    use std::fmt::Write;
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { encoded.push(char::from(b)); }
        else { let _ = write!(encoded, "%{b:02X}"); }
    }
    encoded
}
fn safe_remote(url: &str) -> bool { url.starts_with("https://") && !url.chars().any(char::is_control) }
#[derive(Deserialize)]
struct Auth { access_token: String, expires_in: u64, token_type: String }
#[derive(Deserialize)]
struct SgResponse { success: bool, #[serde(default)] data: Vec<SgArt> }
#[derive(Deserialize)]
struct SgArt { url: String, author: Option<Named> }
#[derive(Deserialize)]
struct Named { name: String }
const GAME_FIELDS: &str = "id,name,summary,first_release_date,genres.name,involved_companies.developer,involved_companies.publisher,involved_companies.company.name,cover.image_id,artworks.image_id,screenshots.image_id";
#[derive(Deserialize)]
struct ExternalGame { uid: String, external_game_source: u64, game: IgGame }
#[derive(Deserialize)]
struct IgGame {
    id: u64,
    name: String,
    #[serde(default)] summary: String,
    first_release_date: Option<i64>,
    #[serde(default)] genres: Vec<Named>,
    #[serde(default)] involved_companies: Vec<Company>,
    cover: Option<IgImage>,
    #[serde(default)] artworks: Vec<IgImage>,
    #[serde(default)] screenshots: Vec<IgImage>,
}
#[derive(Deserialize)]
struct Company { #[serde(default)] developer: bool, #[serde(default)] publisher: bool, company: Named }
#[derive(Deserialize)]
struct IgImage { image_id: String }
impl IgGame {
    fn metadata(self) -> Metadata {
        let mut artwork = Vec::new();
        let mut add = |image: IgImage, kind, size| {
            if !image.image_id.is_empty() && image.image_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                artwork.push(Artwork { kind, url: format!("https://images.igdb.com/igdb/image/upload/t_{size}/{}.jpg", image.image_id), credit: IGDB_CREDIT.into() });
            }
        };
        if let Some(cover) = self.cover { add(cover, Kind::Cover, "cover_big_2x"); }
        for art in self.artworks { add(art, Kind::Hero, "1080p"); }
        for art in self.screenshots { add(art, Kind::Header, "screenshot_big"); }
        Metadata {
            external: vec![ExternalId::new(ExternalId::IGDB, self.id.to_string()).expect("numeric IGDB id")],
            title: self.name, summary: self.summary, release_date: self.first_release_date,
            genres: self.genres.into_iter().map(|g| g.name).collect(),
            developers: self.involved_companies.iter().filter(|c| c.developer).map(|c| c.company.name.clone()).collect(),
            publishers: self.involved_companies.into_iter().filter(|c| c.publisher).map(|c| c.company.name).collect(),
            artwork, credit: IGDB_CREDIT.into(),
        }
    }
}

#[derive(Default)]
struct Budget { starts: VecDeque<Instant>, active: usize }
impl Budget {
    fn available(&mut self, now: Instant) -> bool {
        while self.starts.front().is_some_and(|start| now.duration_since(*start) >= Duration::from_secs(1)) { self.starts.pop_front(); }
        self.active < 8 && self.starts.len() < 4
    }
}
#[derive(Default)]
struct Gate { budget: Mutex<Budget>, wake: Condvar }
impl Gate {
    fn enter(&self) -> Result<Permit<'_>> {
        let mut budget = self.budget.lock().map_err(|_| Error::WorkerClosed)?;
        loop {
            let now = Instant::now();
            if budget.available(now) {
                budget.active += 1;
                budget.starts.push_back(now);
                return Ok(Permit(self));
            }
            let delay = if budget.active >= 8 { Duration::from_secs(1) }
                else { budget.starts.front().map(|t| (*t + Duration::from_secs(1)).saturating_duration_since(now)).unwrap_or(Duration::from_secs(1)) };
            budget = self.wake.wait_timeout(budget, delay).map_err(|_| Error::WorkerClosed)?.0;
        }
    }
}
struct Permit<'a>(&'a Gate);
impl Drop for Permit<'_> {
    fn drop(&mut self) {
        if let Ok(mut budget) = self.0.budget.lock() { budget.active -= 1; self.0.wake.notify_all(); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sliding_window_and_concurrency_are_independent() {
        let now = Instant::now();
        let mut budget = Budget { starts: VecDeque::from([now; 4]), active: 0 };
        assert!(!budget.available(now + Duration::from_millis(999)));
        assert!(budget.available(now + Duration::from_secs(1)));
        budget.active = 8;
        assert!(!budget.available(now + Duration::from_secs(2)));
        budget.active = 7;
        assert!(budget.available(now + Duration::from_secs(2)));
    }
}
