//! What a game's page knows beyond its artwork: the sections it offers (only those with something to show), the owner's own
//! achievements from an engine that can read them, and the quiet look-ahead that fetches a game's page data while its tile rests in focus.

use super::{Command, GameDetails, Media, Model, Request, ipc};
use lumagame_core::{engine::Feature, ids::{EngineId, GameId, ProviderGameId, TargetId}};
use lumagame_meta::hub::{Deck, Hub};

/// How long a tile must stay focused before its page data is fetched ahead of the owner opening it.
const REST_MS: u64 = 700;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubTab {
    Overview,
    Ways,
    Achievements,
    News,
    Media,
    Details,
}
impl HubTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Ways => "Ways to play",
            Self::Achievements => "Achievements",
            Self::News => "News",
            Self::Media => "Media",
            Self::Details => "Details",
        }
    }
}

impl GameDetails {
    /// The page data a game's public sources gave. Nothing is invented: a part the sources did not give stays empty, and the page hides it.
    pub fn from_hub(hub: Hub) -> Self {
        let year = hub.release.as_deref().and_then(|r| r.rsplit(|c: char| !c.is_ascii_digit()).find(|s| s.len() == 4)).and_then(|y| y.parse().ok());
        let mut capabilities = Vec::new();
        match hub.deck {
            Some(Deck::Verified) => capabilities.push("Steam Deck Verified".to_owned()),
            Some(Deck::Playable) => capabilities.push("Steam Deck Playable".to_owned()),
            _ => {}
        }
        capabilities.extend(hub.controller.clone());
        capabilities.extend(["Ultrawide", "HDR", "4K", "Adaptive triggers"].into_iter().filter(|label| hub.facts.iter().any(|f| f.label == *label && f.value == "Yes")).map(str::to_owned));
        let players: Vec<&str> = [("Single-player", "Single-player"), ("Multi-player", "Multiplayer"), ("Co-op", "Co-op")]
            .into_iter()
            .filter(|(category, _)| hub.categories.iter().any(|c| c.starts_with(category)))
            .map(|(_, label)| label)
            .collect();
        let trailers = hub.trailers.iter().map(|t| Media { title: t.title.clone(), image: t.thumb.clone(), video: Some(t.url.clone()) });
        let shots = hub.screenshots.iter().enumerate().map(|(i, url)| Media { title: format!("Screenshot {}", i + 1), image: url.clone(), video: None });
        Self {
            description: hub.description.clone(),
            year,
            genres: hub.genres.clone(),
            players: (!players.is_empty()).then(|| players.join(", ")),
            developer: (!hub.developers.is_empty()).then(|| hub.developers.join(", ")),
            capabilities,
            cloud_saves: hub.categories.iter().any(|c| c == "Steam Cloud").then_some(true),
            media: trailers.chain(shots).collect(),
            hub,
        }
    }
}

impl Model {
    /// The sections the open game's page offers, in order. A section with nothing to show is not offered.
    pub fn hub_tabs(&self) -> Vec<HubTab> {
        let mut tabs = vec![HubTab::Overview, HubTab::Ways];
        let Some(id) = self.view.selected else { return tabs };
        let details = self.details.get(&id);
        if details.is_some_and(|d| d.hub.achievements.total > 0) || self.owned.get(&id).is_some_and(|u| !u.is_empty()) {
            tabs.push(HubTab::Achievements);
        }
        if details.is_some_and(|d| !d.hub.news.is_empty()) {
            tabs.push(HubTab::News);
        }
        if details.is_some_and(|d| !d.media.is_empty()) {
            tabs.push(HubTab::Media);
        }
        if details.is_some_and(|d| {
            let h = &d.hub;
            !(h.developers.is_empty() && h.publishers.is_empty() && h.release.is_none() && h.categories.is_empty() && h.facts.is_empty() && h.deck.is_none() && h.credits.is_empty())
        }) {
            tabs.push(HubTab::Details);
        }
        tabs
    }

    pub fn hub_tab(&self) -> HubTab {
        self.hub_tabs().get(self.view.hub_tab).copied().unwrap_or(HubTab::Overview)
    }

    /// What the owner played of the game in the last two weeks, in seconds, where a service says so.
    pub fn recent_play(&self, id: GameId) -> Option<u64> {
        lumagame_core::activity::recent_play(self.game(id)?)
    }

    /// The owner's own unlocked achievements in the game, when an engine has said.
    pub fn unlocked(&self, id: GameId) -> Option<&[ipc::Unlocked]> {
        self.owned.get(&id).map(Vec::as_slice)
    }

    /// Asks the engine that holds the game for the owner's own achievements, once until the app comes back from the background.
    pub(super) fn request_owned(&mut self, id: GameId, out: &mut Vec<Command>) {
        let Some(game) = self.game(id) else { return };
        let Some(source) = game.sources.iter().find(|s| {
            self.enabled(&s.engine)
                && self.integration(&s.engine).is_some_and(|e| {
                    e.descriptor.as_ref().is_some_and(|d| d.capabilities.has(Feature::Achievements)) && e.targets.iter().any(|t| t.id == s.target && t.link == ipc::Link::Linked)
                })
        }) else {
            return;
        };
        let request = Request::Achievements { target: source.target.clone(), game: source.entry.id.clone() };
        let engine = source.engine.clone();
        if self.requested_owned.insert(id) {
            Self::send(out, &engine, request);
        }
    }

    pub(super) fn owned_achievements(&mut self, engine: &EngineId, target: &TargetId, game: &ProviderGameId, unlocked: Vec<ipc::Unlocked>) {
        let found = self.games.iter().find(|g| g.sources.iter().any(|s| s.engine == *engine && s.target == *target && s.entry.id == *game)).map(|g| g.id);
        if let Some(id) = found {
            self.owned.insert(id, unlocked);
        }
    }

    /// A tile that stays focused for a moment gets its page data fetched, so the page is ready when it opens.
    pub(super) fn look_ahead(&mut self, out: &mut Vec<Command>) {
        let Some(id) = self.view.featured.filter(|_| matches!(self.view.page, super::Page::Home | super::Page::Library)) else {
            self.resting = None;
            return;
        };
        match self.resting {
            Some((rested, since)) if rested == id => {
                if self.now_ms.saturating_sub(since) >= REST_MS && !self.details.contains_key(&id) && self.requested_details.insert(id) {
                    out.push(Command::FetchDetails { game: id });
                }
            }
            _ => self.resting = Some((id, self.now_ms)),
        }
    }
}
