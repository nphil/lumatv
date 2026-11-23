//! Refreshing the libraries of the signed-in services: the owner's "Refresh libraries", and the quiet refreshes the app does by itself.
//!
//! The owner's refresh is a run: it asks each eligible engine (an account engine such as Steam, or a cloud engine that re-checks the
//! stores linked to it, GeForce NOW) and shows honest progress while it waits. When every engine has answered it compares the games
//! before and after, and says "N new games", "Up to date" or what went wrong, in plain words. The result clears itself after a short
//! while, so no notice stays lit on the panel.
//!
//! The automatic refreshes (stream return, coming back after a long time away) never show anything and are throttled per engine.

use super::{Command, Focus, HomeItem, Integration, Model, Page, Request, ipc};
use lumagame_core::engine::{EngineKind, Feature};
use lumagame_core::ids::{EngineId, GameId, TargetId};
use std::collections::{BTreeMap, BTreeSet};

/// The least time between two automatic refreshes of one engine.
pub const AUTO_MIN_MS: u64 = 300_000;
/// How long the app must have been in the background for coming back to refresh.
pub const AWAY_MS: u64 = 30 * 60_000;
/// How long a result stays on screen.
const RESULT_MS: u64 = 12_000;
/// How long a refresh waits for an engine before it reports the engine as too slow.
const PATIENCE_MS: u64 = 180_000;
/// Progress shows the seconds it has waited once it has waited this long.
const SECONDS_AFTER_MS: u64 = 3_000;

#[derive(Clone, Debug, Default)]
pub struct Refresher {
    run: Option<Run>,
    result: Option<Outcome>,
    /// When each engine's library last finished loading, by the app's clock.
    fetched: BTreeMap<EngineId, u64>,
    /// When the app went to the background.
    away_since: Option<u64>,
}

#[derive(Clone, Debug)]
struct Run {
    before: BTreeSet<GameId>,
    since_ms: u64,
    engines: BTreeMap<EngineId, Stage>,
    notes: Vec<String>,
    problems: Vec<String>,
}

#[derive(Clone, Debug)]
enum Stage {
    /// The engine is re-checking the linked stores and says what it is doing.
    Syncing(String),
    /// The library is being fetched.
    Fetching,
}

#[derive(Clone, Debug)]
struct Outcome {
    text: String,
    problem: bool,
    until_ms: u64,
}

/// What the screens show about a refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line {
    Working { text: String, seconds: Option<u64> },
    Done { text: String, problem: bool },
}

/// The held card: what the focus pointed at before the catalog was rebuilt.
pub enum Held {
    Grid(GameId, usize),
    Home(usize, HomeItem),
}

impl Refresher {
    /// Whether the clock moved something on screen.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        let before = self.result.is_some();
        self.result = self.result.take().filter(|r| now_ms < r.until_ms);
        before != self.result.is_some()
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

impl Model {
    /// When `engine`'s library last finished loading, by the app's clock.
    pub fn fetched_at(&self, engine: &EngineId) -> Option<u64> {
        self.refresher.fetched.get(engine).copied()
    }
    /// The progress or the result to show, if any.
    pub fn refresh_line(&self) -> Option<Line> {
        if let Some(run) = &self.refresher.run {
            let text = run.engines.values().next().map_or_else(
                || "Refreshing libraries".to_owned(),
                |stage| match stage {
                    Stage::Syncing(step) => step.clone(),
                    Stage::Fetching => "Loading your games".to_owned(),
                },
            );
            let waited = self.now_ms.saturating_sub(run.since_ms);
            return Some(Line::Working { text, seconds: (waited >= SECONDS_AFTER_MS).then_some(waited / 1000) });
        }
        self.refresher.result.as_ref().map(|r| Line::Done { text: r.text.clone(), problem: r.problem })
    }

    /// How long ago the latest library finished loading; none before the first.
    pub fn libraries_age_ms(&self) -> Option<u64> {
        self.refresher.fetched.values().max().map(|at| self.now_ms.saturating_sub(*at))
    }

    pub fn refreshing(&self) -> bool {
        self.refresher.run.is_some()
    }

    /// The engines a refresh can reach: enabled, ready, with an account linked, and either an account engine or one that syncs stores.
    fn refreshable(&self, only: Option<&EngineId>) -> Vec<(EngineId, Vec<TargetId>)> {
        self.integrations
            .iter()
            .filter(|e| only.is_none_or(|id| *id == e.manifest.id) && self.can_refresh(e))
            .map(|e| (e.manifest.id.clone(), e.targets.iter().filter(|t| t.link == ipc::Link::Linked).map(|t| t.id.clone()).collect()))
            .collect()
    }

    /// An integration has a library the owner's refresh can reload.
    pub fn can_refresh(&self, e: &Integration) -> bool {
        self.enabled(&e.manifest.id)
            && e.descriptor.as_ref().is_some_and(|d| e.manifest.kind == EngineKind::Account || d.capabilities.has(Feature::StoreSync))
            && e.targets.iter().any(|t| t.link == ipc::Link::Linked)
    }

    fn syncs(&self, id: &EngineId) -> bool {
        self.integration(id).and_then(|e| e.descriptor.as_ref()).is_some_and(|d| d.capabilities.has(Feature::StoreSync))
    }

    fn name_of(&self, id: &EngineId) -> String {
        self.integration(id).map_or_else(|| id.to_string(), |e| e.manifest.name.clone())
    }

    fn loading(&self, id: &EngineId) -> bool {
        self.pages.keys().any(|(engine, _)| engine == id)
    }

    /// The owner's "Refresh libraries": every reachable engine, or only `only`. One refresh runs at a time; pressing again while it runs
    /// changes nothing, because the screen already says what it is doing.
    pub fn refresh_libraries(&mut self, only: Option<EngineId>, out: &mut Vec<Command>) {
        if self.refresher.run.is_some() {
            return;
        }
        self.refresher.result = None;
        let eligible = self.refreshable(only.as_ref());
        if eligible.is_empty() {
            let text = match &only {
                Some(id) => format!("{} has no account linked. Sign in to refresh it.", self.name_of(id)),
                None => "Nothing to refresh. Sign in to GeForce NOW or Steam in Settings.".to_owned(),
            };
            self.refresher.result = Some(Outcome { text, problem: true, until_ms: self.now_ms + RESULT_MS });
            return;
        }
        let mut run = Run {
            before: self.games.iter().map(|g| g.id).collect(),
            since_ms: self.now_ms,
            engines: BTreeMap::new(),
            notes: Vec::new(),
            problems: Vec::new(),
        };
        for (engine, targets) in eligible {
            if self.loading(&engine) {
                // A fetch already under way is the refresh: a second first-page request would collide with its cursor.
                run.engines.insert(engine, Stage::Fetching);
                continue;
            }
            Self::send(out, &engine, Request::Action { key: "refresh".into() });
            if self.syncs(&engine) {
                let name = self.name_of(&engine);
                run.engines.insert(engine, Stage::Syncing(format!("Asking {name} to check your stores")));
            } else {
                for target in targets {
                    self.pages.insert((engine.clone(), target.clone()), Vec::new());
                    Self::send(out, &engine, Request::Library { target, cursor: None });
                }
                run.engines.insert(engine, Stage::Fetching);
            }
        }
        self.refresher.run = Some(run);
    }

    /// Asks each signed-in account engine for its library again; the quiet refresh. After a stream it shows the play that just ended
    /// on Home at once, and an engine asked less than [`AUTO_MIN_MS`] ago is left alone. `force` is the engine's own "skip the saved
    /// copy" and is only for callers that want the network. A fetch already under way is left to finish.
    pub(crate) fn refresh_accounts(&mut self, force: bool, out: &mut Vec<Command>) {
        let wanted: Vec<(EngineId, Vec<TargetId>)> = self
            .refreshable(None)
            .into_iter()
            .filter(|(id, _)| force || self.refresher.fetched.get(id).is_none_or(|at| self.now_ms.saturating_sub(*at) >= AUTO_MIN_MS))
            .collect();
        for (engine, targets) in wanted {
            let account = self.integration(&engine).is_some_and(|e| e.manifest.kind == EngineKind::Account);
            for target in targets {
                let std::collections::btree_map::Entry::Vacant(slot) = self.pages.entry((engine.clone(), target.clone())) else { continue };
                slot.insert(Vec::new());
                if force && account {
                    Self::send(out, &engine, Request::Action { key: "refresh".into() });
                }
                Self::send(out, &engine, Request::Library { target, cursor: None });
            }
        }
    }

    /// The app left or came back. Coming back after [`AWAY_MS`] refreshes the libraries quietly.
    pub(super) fn refresh_foreground(&mut self, on: bool, out: &mut Vec<Command>) {
        if !on {
            self.refresher.away_since.get_or_insert(self.now_ms);
            return;
        }
        if let Some(since) = self.refresher.away_since.take()
            && self.now_ms.saturating_sub(since) >= AWAY_MS
            && !self.session_active()
        {
            self.refresh_accounts(false, out);
        }
    }

    /// The clock: the result clears itself and a refresh that no engine finishes is ended. `second` is a new second since the last tick.
    /// True when the screen changes.
    pub(super) fn refresh_tick(&mut self, second: bool) -> bool {
        let now = self.now_ms;
        let mut changed = self.refresher.tick(now);
        if self.refresher.run.as_ref().is_some_and(|run| now.saturating_sub(run.since_ms) >= PATIENCE_MS) {
            let slow: Vec<EngineId> = self.refresher.run.as_ref().map(|run| run.engines.keys().cloned().collect()).unwrap_or_default();
            for engine in slow {
                let name = self.name_of(&engine);
                self.refresh_engine_over(&engine, Some(format!("{name} is taking too long. Try again later.")));
            }
            changed = true;
        }
        // The seconds of progress count up.
        changed | (second && self.refresher.run.is_some())
    }

    /// An engine's step or the end of its part of the refresh.
    pub(super) fn refresh_step(&mut self, id: &EngineId, step: Option<String>, note: Option<String>, out: &mut Vec<Command>) {
        let Some(run) = self.refresher.run.as_mut() else { return };
        let Some(stage) = run.engines.get_mut(id) else { return };
        match step {
            Some(text) => *stage = Stage::Syncing(text),
            None => {
                *stage = Stage::Fetching;
                run.notes.extend(note);
                let targets: Vec<TargetId> = self
                    .integration(id)
                    .map(|e| e.targets.iter().filter(|t| t.link == ipc::Link::Linked).map(|t| t.id.clone()).collect())
                    .unwrap_or_default();
                if targets.is_empty() {
                    return self.refresh_engine_over(id, None);
                }
                for target in targets {
                    // The first page restarts the engine's traversal, so any page still on its way from an older one is dropped with it.
                    self.pages.insert((id.clone(), target.clone()), Vec::new());
                    Self::send(out, id, Request::Library { target, cursor: None });
                }
            }
        }
    }

    /// An engine's library finished loading.
    pub(super) fn refresh_loaded(&mut self, id: &EngineId) {
        self.refresher.fetched.insert(id.clone(), self.now_ms);
        if !self.loading(id) && self.refresher.run.as_ref().is_some_and(|run| matches!(run.engines.get(id), Some(Stage::Fetching))) {
            self.refresh_engine_over(id, None);
        }
    }

    /// An engine reported an error: in a refresh it ends that engine's part, with the reason in plain words.
    pub(super) fn refresh_error(&mut self, id: &EngineId, code: ipc::ErrorCode) {
        if !self.refresher.run.as_ref().is_some_and(|run| run.engines.contains_key(id)) || code == ipc::ErrorCode::Unsupported {
            return;
        }
        let name = self.name_of(id);
        let problem = match code {
            ipc::ErrorCode::Network => format!("Can't reach {name}. Check the network."),
            ipc::ErrorCode::SignIn => format!("{name} needs you to sign in again."),
            _ => format!("{name} could not be refreshed."),
        };
        self.refresh_engine_over(id, Some(problem));
    }

    /// The engine's process ended.
    pub(super) fn refresh_gone(&mut self, id: &EngineId) {
        if self.refresher.run.as_ref().is_some_and(|run| run.engines.contains_key(id)) {
            let name = self.name_of(id);
            self.refresh_engine_over(id, Some(format!("The connection to {name} was lost.")));
        }
    }

    fn refresh_engine_over(&mut self, id: &EngineId, problem: Option<String>) {
        let Some(run) = self.refresher.run.as_mut() else { return };
        run.engines.remove(id);
        run.problems.extend(problem);
        if !run.engines.is_empty() {
            return;
        }
        let Some(run) = self.refresher.run.take() else { return };
        let after: BTreeSet<GameId> = self.games.iter().map(|g| g.id).collect();
        let (added, removed) = (after.difference(&run.before).count(), run.before.difference(&after).count());
        let changes = match (added, removed) {
            (0, 0) => "Up to date.".to_owned(),
            (a, 0) => format!("{}.", plural(a, "new game", "new games")),
            (0, r) => format!("{} removed.", plural(r, "game", "games")),
            (a, r) => format!("{}, {} removed.", plural(a, "new game", "new games"), r),
        };
        let text = match (run.problems.is_empty(), added + removed) {
            (false, 0) => run.problems.join(" "),
            _ => std::iter::once(changes).chain(run.problems.iter().cloned()).chain(run.notes.iter().cloned()).collect::<Vec<_>>().join(" "),
        };
        let problem = !run.problems.is_empty() || !run.notes.is_empty();
        self.refresher.result = Some(Outcome { text, problem, until_ms: self.now_ms + RESULT_MS });
    }

    /// What the focus points at, so that it can follow its game when a refresh moves the games around.
    pub(super) fn held(&self) -> Option<Held> {
        match (self.view.page, self.view.focus) {
            (Page::Library, Focus::Grid(i)) => self.filtered.get(i).map(|id| Held::Grid(*id, self.view.first_grid_row.min(i / self.prefs.covers.columns()))),
            (Page::Home, Focus::Home(r, i)) => self.home_rows.get(r).and_then(|row| row.items.get(i)).map(|item| Held::Home(r, item.clone())),
            _ => None,
        }
    }

    /// Puts the focus back on what it pointed at before the catalog was rebuilt, wherever that card is now.
    pub(super) fn follow(&mut self, held: Option<Held>) {
        match held {
            Some(Held::Grid(id, first_row)) => {
                if let Some(at) = self.filtered.iter().position(|g| *g == id) {
                    let columns = self.prefs.covers.columns();
                    let offset = match self.view.focus {
                        Focus::Grid(old) => (old / columns).saturating_sub(first_row),
                        _ => 0,
                    };
                    self.view.focus = Focus::Grid(at);
                    self.view.first_grid_row = (at / columns).saturating_sub(offset);
                }
            }
            Some(Held::Home(row, item)) => {
                let found = self.home_rows.get(row).and_then(|r| r.items.iter().position(|i| *i == item)).map(|i| (row, i)).or_else(|| {
                    self.home_rows.iter().enumerate().find_map(|(r, row)| row.items.iter().position(|i| *i == item).map(|i| (r, i)))
                });
                if let Some((r, i)) = found {
                    self.view.focus = Focus::Home(r, i);
                    self.view.home_row = r;
                }
            }
            None => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use crate::game::model::{tests::*, *};
    #[test]
    fn a_refresh_counts_new_and_removed_games_keeps_the_focused_game_and_clears_its_notice() {
        use lumagame_core::activity::Activity;
        let (mut m, account) = refreshing_steam();
        m.view.page = Page::Library;
        m.view.focus = Focus::Grid(1);
        m.rebuild_navigation();
        let held = m.filtered[1];
        assert_eq!(m.games()[..].iter().find(|g| g.id == held).map(|g| g.title.as_str()), Some("Delta"));

        let mut out = Vec::new();
        m.refresh_libraries(None, &mut out);
        assert!(matches!(sent(&out)[..], [Request::Action { .. }, Request::Library { cursor: None, .. }]));
        assert!(matches!(m.refresh_line(), Some(RefreshLine::Working { seconds: None, .. })), "no seconds while it is quick");
        let mut again = Vec::new();
        m.refresh_libraries(None, &mut again);
        assert!(again.is_empty(), "pressing it while it runs changes nothing");

        let records = [("a0", "Alpha"), ("0", "Beta"), ("2", "Delta")].iter().map(|(i, t)| entry(i, t, ipc::Role::Game, &[&format!("steam:{i}")], Activity::default())).collect();
        engine_says(&mut m, "steam", ipc::Event::Library { target: account, entries: records, next: None });
        assert_eq!(m.refresh_line(), Some(RefreshLine::Done { text: "1 new game, 1 removed.".into(), problem: false }));
        assert_eq!(m.view.focus, Focus::Grid(m.filtered.iter().position(|id| m.game(*id).unwrap().title == "Delta").unwrap()), "the focus follows its game");

        m.step(Input::Tick { now_ms: 11_999, unix_s: 0 });
        assert!(m.refresh_line().is_some());
        m.step(Input::Tick { now_ms: 12_000, unix_s: 0 });
        assert_eq!(m.refresh_line(), None, "the result clears itself");
    }
    #[test]
    fn a_refresh_that_finds_nothing_says_so_and_a_slow_one_counts_seconds() {
        let (mut m, account) = refreshing_steam();
        let mut out = Vec::new();
        m.refresh_libraries(None, &mut out);
        m.step(Input::Tick { now_ms: 4_000, unix_s: 0 });
        assert!(matches!(m.refresh_line(), Some(RefreshLine::Working { seconds: Some(4), .. })));
        let same = ["Beta", "Gamma", "Delta"].iter().enumerate().map(|(i, t)| entry(&format!("{i}"), t, ipc::Role::Game, &[&format!("steam:{i}")], Default::default())).collect();
        engine_says(&mut m, "steam", ipc::Event::Library { target: account, entries: same, next: None });
        assert_eq!(m.refresh_line(), Some(RefreshLine::Done { text: "Up to date.".into(), problem: false }));
    }
    #[test]
    fn a_refresh_names_a_network_or_sign_in_failure_in_plain_words() {
        let (mut m, _) = refreshing_steam();
        for (code, text) in [(ipc::ErrorCode::Network, "Can't reach Steam. Check the network."), (ipc::ErrorCode::SignIn, "Steam needs you to sign in again.")] {
            m.refresh_libraries(None, &mut Vec::new());
            engine_says(&mut m, "steam", ipc::Event::Error { code, message: "x".into() });
            assert_eq!(m.refresh_line(), Some(RefreshLine::Done { text: text.into(), problem: true }));
            assert!(!m.refreshing());
        }
        let mut nothing = Model::new(State::default(), Listing::default(), vec![manifest("pc", "PC", EngineKind::Host)]);
        nothing.refresh_libraries(None, &mut Vec::new());
        assert!(matches!(nothing.refresh_line(), Some(RefreshLine::Done { problem: true, .. })), "no account: it says to sign in");
    }
    #[test]
    fn a_cloud_refresh_waits_for_the_engines_store_check_then_loads_the_library() {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud)]);
        describe(&mut m, "gfn", vec![lumagame_core::engine::AuthMethod::Qr], &[Feature::StoreSync]);
        let account = TargetId::new("me").unwrap();
        engine_says(&mut m, "gfn", ipc::Event::Targets { targets: vec![linked("me")] });
        engine_says(&mut m, "gfn", ipc::Event::Library { target: account.clone(), entries: Vec::new(), next: None });

        let mut out = Vec::new();
        m.refresh_libraries(Some(EngineId::new("gfn").unwrap()), &mut out);
        assert!(matches!(sent(&out)[..], [Request::Action { .. }]), "the library waits for the stores' answer");
        engine_says(&mut m, "gfn", ipc::Event::Refresh { step: Some("Checking Steam".into()), note: None });
        assert_eq!(m.refresh_line(), Some(RefreshLine::Working { text: "Checking Steam".into(), seconds: None }));
        let asked = engine_says(&mut m, "gfn", ipc::Event::Refresh { step: None, note: Some("Epic has not answered.".into()) });
        assert!(matches!(sent(&asked)[..], [Request::Library { cursor: None, .. }]));
        let found = vec![entry("g1", "Fortnite", ipc::Role::Game, &[], Default::default())];
        engine_says(&mut m, "gfn", ipc::Event::Library { target: account, entries: found, next: None });
        assert_eq!(m.refresh_line(), Some(RefreshLine::Done { text: "1 new game. Epic has not answered.".into(), problem: true }));
    }
    #[test]
    fn coming_back_after_half_an_hour_refreshes_quietly_and_a_short_absence_does_not() {
        let (mut m, _) = refreshing_steam();
        m.step(Input::Tick { now_ms: 400_000, unix_s: 0 });
        m.step(Input::Foreground(false));
        m.step(Input::Tick { now_ms: 400_000 + 600_000, unix_s: 0 });
        assert!(sent(&m.step(Input::Foreground(true))).is_empty(), "ten minutes away is not enough");
        m.step(Input::Foreground(false));
        m.step(Input::Tick { now_ms: 1_000_000 + 1_800_000, unix_s: 0 });
        let out = m.step(Input::Foreground(true));
        assert!(matches!(sent(&out)[..], [Request::Library { cursor: None, .. }]));
        assert_eq!(m.refresh_line(), None, "a quiet refresh shows nothing");
    }
}
