use anyhow::{Context, Result, ensure};
use lumagame_core::{ids::{ExternalId, ProviderGameId}, ipc::{Art, LibraryEntry, Readiness, Role}};
use pf_client_core::library::{GameEntry, RunningGame};
use punktfunk_core::tls::{PinVerify, pinned_builder, ureq_agent};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use serde::Deserialize;
use std::{collections::BTreeSet, sync::Arc, time::Duration};
use crate::store::Host;

pub const DESKTOP: &str = "lumagame:desktop";

#[derive(Deserialize)]
struct Title {
    #[serde(flatten)]
    game: GameEntry,
    #[serde(default, alias = "steam_app_id")]
    steam_appid: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct Page { items: Vec<Title>, #[serde(default)] next_cursor: Option<String> }

pub struct Library {
    agent: ureq::Agent,
    base: String,
}
impl Library {
    pub fn new(host: &Host, identity: &(String, String)) -> Result<Self> {
        let tls = pinned_builder(PinVerify::new(Some(host.pin()?)))?
            .with_client_auth_cert(vec![CertificateDer::from_pem_slice(identity.0.as_bytes())?], PrivateKeyDer::from_pem_slice(identity.1.as_bytes())?)?;
        let config = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(3)))
            .timeout_global(Some(Duration::from_secs(8))).max_redirects(0).build();
        Ok(Self { agent: ureq_agent::agent(Arc::new(tls), config), base: pf_client_core::library::base_url(&host.known.addr, host.known.effective_mgmt_port()) })
    }
    pub fn page(&self, cursor: Option<&str>) -> Result<(Vec<LibraryEntry>, Option<String>)> {
        ensure!(cursor.is_none_or(|c| c.len() <= 4101), "Invalid library cursor");
        if let Some(offset)=cursor.and_then(|c|c.strip_prefix("legacy:")) {return self.legacy(offset.parse().context("Invalid library cursor")?);}
        let host_cursor=cursor.map(|c|c.strip_prefix("page:").context("Invalid library cursor")).transpose()?;
        let mut req = self.agent.get(format!("{}/api/v1/library/page", self.base)).query("limit", "100");
        if let Some(cursor) = host_cursor { req = req.query("cursor", cursor); }
        let mut response = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::StatusCode(404)) if host_cursor.is_none() => return self.legacy(0),
            Err(e) => return Err(e.into()),
        };
        let page: Page = serde_json::from_str(&response.body_mut().with_config().limit(1 << 20).read_to_string()?)?;
        ensure!(page.items.len() <= 100, "The host returned an oversized library page.");
        ensure!(page.next_cursor.as_deref().is_none_or(|c| !c.is_empty() && c.len() <= 4096 && Some(c) != host_cursor), "The host repeated its library cursor.");
        let mut entries = convert(page.items)?;
        if cursor.is_none() { entries.insert(0, desktop()); }
        Ok((entries, page.next_cursor.map(|c|format!("page:{c}"))))
    }
    fn legacy(&self,offset:usize)->Result<(Vec<LibraryEntry>,Option<String>)> {
        let mut response=self.agent.get(format!("{}/api/v1/library",self.base)).call()?;
        let titles:Vec<Title>=serde_json::from_str(&response.body_mut().with_config().limit(8<<20).read_to_string()?)?;
        ensure!(offset<=titles.len(),"The host's library changed; refresh it.");
        let end=offset.saturating_add(100).min(titles.len());
        let next=(end<titles.len()).then(||format!("legacy:{end}"));
        let mut entries=convert(titles.into_iter().skip(offset).take(100).collect())?;
        if offset==0 {entries.insert(0,desktop());}
        Ok((entries,next))
    }
    pub fn check(&self, game: &ProviderGameId) -> Result<Readiness> {
        let mut cursor = None;
        let mut seen = BTreeSet::new();
        for _ in 0..500 {
            let (entries, next) = self.page(cursor.as_deref())?;
            if let Some(e) = entries.iter().find(|e| e.id == *game) { return Ok(e.readiness); }
            let Some(next) = next else { return Ok(Readiness::Install); };
            ensure!(seen.insert(next.clone()), "The host's library cursor looped.");
            cursor = Some(next);
        }
        anyhow::bail!("The host's library is too large.")
    }
    pub fn end_game(&self, game: &ProviderGameId) -> Result<()> {
        ensure!(game.as_str() != DESKTOP, "The desktop is not a game to close.");
        let body = serde_json::json!({"app_id": game.as_str(), "streaming": true}).to_string();
        self.agent.post(format!("{}/api/v1/game/end", self.base)).header("Content-Type", "application/json").send(body.as_str())?;
        Ok(())
    }
    pub fn running(&self) -> Result<Vec<String>> {
        #[derive(Deserialize)] struct Status { #[serde(default)] games: Vec<RunningGame> }
        let mut response = self.agent.get(format!("{}/api/v1/status", self.base)).call()?;
        let status: Status = serde_json::from_str(&response.body_mut().with_config().limit(1 << 20).read_to_string()?)?;
        Ok(status.games.into_iter().filter(|g| g.is_up()).filter_map(|g| g.app_id).collect())
    }
}

fn desktop() -> LibraryEntry {
    LibraryEntry { id: ProviderGameId::new(DESKTOP).unwrap(), title: "Desktop".into(), role: Role::Launcher, external: Vec::new(), store: None,
        store_account: None, art: Art::default(), readiness: Readiness::Ready, resumable: false }
}
fn steam_id(t: &Title) -> Option<String> {
    let id = t.game.id.strip_prefix("steam:").map(str::to_owned).or_else(|| match t.steam_appid.as_ref()? {
        serde_json::Value::Number(n) => n.as_u64().map(|n| n.to_string()),
        serde_json::Value::String(s) => Some(s.clone()), _ => None,
    })?;
    (id.parse::<u32>().ok()? > 0 && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}
fn https(url: Option<String>) -> Option<String> { url.filter(|u| u.starts_with("https://") && u.len() <= 2048) }
fn convert(titles: Vec<Title>) -> Result<Vec<LibraryEntry>> {
    titles.into_iter().map(|t| {
        let steam = steam_id(&t);
        let role = if t.game.is_launcher() { Role::Launcher } else { Role::Game };
        let mut art = Art { cover: https(t.game.art.portrait), hero: https(t.game.art.hero), ..Default::default() };
        if let Some(id) = &steam {
            art.cover = Some(format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/library_600x900.jpg"));
            art.hero = Some(format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{id}/library_hero.jpg"));
        }
        let readiness = if t.game.install.as_ref().is_some_and(|i| i.missing()) { Readiness::Install } else { Readiness::Ready };
        ensure!(!t.game.title.is_empty() && t.game.title.len() <= 1024, "Invalid host title");
        ensure!(t.game.store.len() <= 128, "Invalid host store name");
        Ok(LibraryEntry { id: ProviderGameId::new(t.game.id).context("Invalid host game id")?, title: t.game.title, role,
            external: if role == Role::Game { steam.map(|id| ExternalId::new("steam", id).unwrap()).into_iter().collect() } else { Vec::new() },
            store: Some(t.game.store), store_account: None, art, readiness, resumable: false })
    }).collect()
}

pub fn readiness_error(error: &anyhow::Error, host: &Host) -> Readiness {
    match error.downcast_ref::<ureq::Error>() {
        Some(ureq::Error::StatusCode(401 | 403) | ureq::Error::Rustls(_)) => Readiness::SignIn,
        Some(ureq::Error::Io(_) | ureq::Error::Timeout(_)) => if host.known.mac.is_empty() { Readiness::Offline } else { Readiness::Asleep },
        _ => Readiness::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_ids_remain_launch_handles_and_only_explicit_steam_ids_merge() {
        let titles: Vec<Title> = serde_json::from_str(r#"[
            {"id":"steam:570","store":"steam","title":"Dota"},
            {"id":"custom:abc","store":"custom","title":"Dota","steam_app_id":570},
            {"id":"custom:def","store":"custom","title":"Dota"},
            {"id":"custom:big","store":"custom","title":"Big Picture","role":"launcher","steam_appid":"570","install":{"state":"missing"}}
        ]"#).unwrap();
        let entries = convert(titles).unwrap();
        assert_eq!(entries[0].external, entries[1].external);
        assert_eq!(entries[1].id.as_str(), "custom:abc");
        assert!(entries[2].external.is_empty() && entries[3].external.is_empty());
        assert_eq!(entries[3].readiness, Readiness::Install);
        assert_eq!(entries[3].role, Role::Launcher);
        assert_eq!(desktop().role, Role::Launcher);
    }
}
