//! A session from launch to its end: the launch screen and queue, the running stream, the in-stream menu and ending a session.
use super::nav::Slice;
use super::settings::Place;
use super::*;
use lumagame_core::session::Outcome;

/// The in-stream menu's buttons; `Focus::Overlay(i)` is the `i`th of [`Model::overlay_actions`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayAction {
    Resume,
    Settings,
    Stats,
    /// Over a Steam game whose engine can open Steam's overlay.
    SteamOverlay,
    EndSession,
}
/// The far side's idle countdown while it runs: when it ends by the model's clock, and when a press last told the engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IdleWarning {
    pub ends_ms: u64,
    pub(super) sent_ms: Option<u64>,
}
/// How far a session has moved through the cloud queue: the first position it reported and the lowest since. The share is the people
/// it has got past, so a position that climbs again leaves it where it was, and it never reads as done while the session still waits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QueueWatch(Option<(u32, u32)>);
impl QueueWatch {
    fn observe(&mut self, phase: &Phase) {
        if let Phase::Queue { position: Some(at), .. } = phase {
            let (_, best) = self.0.get_or_insert((*at, *at));
            *best = (*best).min(*at);
        }
    }
    /// The share of the queue got through. `None` unless the session waits at a known position behind someone: a queue that began
    /// at the front has nothing to measure.
    fn share(&self, phase: &Phase) -> Option<f32> {
        let Phase::Queue { position: Some(_), .. } = phase else { return None };
        let (first, best) = self.0?;
        (first > 1).then(|| (first - best) as f32 / first as f32)
    }
}
/// What moved the session towards its end, for diagnostics. It is read from the input that did it, so an engine's own wording never has to be parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause { Owner, Close, Background, Engine, EngineExit, Timeout }
impl Cause {
    pub fn word(self) -> &'static str {
        match self { Cause::Owner => "owner", Cause::Close => "app_close", Cause::Background => "background", Cause::Engine => "engine", Cause::EngineExit => "engine_exit", Cause::Timeout => "timeout" }
    }
}
pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        m.launch_grid(layer)
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        m.launch_activate(f, out);
    }
    fn adjust(_m: &mut Model, _f: Focus, _forward: bool, _out: &mut Vec<Command>) -> bool {
        false
    }
    fn subject(m: &Model) -> Option<GameId> {
        m.session.as_ref().map(|s| s.game)
    }
}
impl Model {
    /// Why the current session of `id` failed, in the engine's words.
    pub fn last_failure(&self, id: GameId) -> Option<&str> {
        let session = self.session.as_ref().filter(|s| s.game == id)?;
        match session.phase() {
            Phase::Done(Outcome::Failed { message, .. }) => Some(message),
            _ => None,
        }
    }
    fn launch_grid(&self, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        match (layer, self.view.page) {
            (Some(Layer::EndSession), _) => vec![vec![Focus::SheetAction(0), Focus::SheetAction(1)]],
            (Some(Layer::StreamMenu), _) => vec![(0..self.overlay_actions().count()).map(Focus::Overlay).collect()],
            (None, Page::Launch) if self.can_cancel() => vec![vec![Focus::Cancel]],
            _ => Vec::new(),
        }
    }
    fn launch_activate(&mut self, f: Focus, out: &mut Vec<Command>) {
        match f {
            Focus::SheetAction(i) if self.view.top() == Some(Layer::EndSession) => {
                if i == 0 {
                    self.pop_layer();
                    self.resume(out);
                } else {
                    self.pop_layer();
                    self.session_input(session::Input::Stop { end_game: self.can_end_game() }, out);
                }
            }
            Focus::Overlay(i) => {
                let action = self.overlay_actions().nth(i);
                match action {
                    Some(OverlayAction::Resume) => self.resume(out),
                    Some(OverlayAction::Settings) => self.open_settings(Place::Picture),
                    Some(OverlayAction::Stats) => self.set_stats(self.prefs.stats.mode.next(), out),
                    Some(OverlayAction::SteamOverlay) => self.open_steam_overlay(out),
                    Some(OverlayAction::EndSession) => self.push_layer(Layer::EndSession, Focus::SheetAction(0)),
                    None => {}
                }
            }
            Focus::Cancel => self.session_input(session::Input::Cancel, out),
            _ => {}
        }
    }
    /// How far the queue has moved the launch, from 0 to 1, when the engine reports a position to measure it by.
    pub fn queue_progress(&self) -> Option<f32> {
        self.session.as_ref().and_then(|s| self.queue.share(s.phase()))
    }
    /// Whether the launch can still be called off: a stop that has begun cannot be.
    pub fn can_cancel(&self) -> bool {
        self.session.as_ref().is_some_and(|s| matches!(s.phase(), Phase::Checking | Phase::SignIn | Phase::Queue { .. } | Phase::Starting))
    }
    /// What the running stream was asked for, which the HUD's diagnosis weighs the readings against.
    pub(super) fn asked(&self) -> crate::game::stats::Asked {
        let stream = self.view.stream.as_ref();
        crate::game::stats::Asked { fps: stream.map_or(self.prefs.fps, |s| s.fps), cap_mbps: self.stream_bitrate_mbps(), sdr_reason: stream.and_then(|s| s.sdr_reason.clone()) }
    }
    pub fn hud_visible(&self) -> bool {
        self.foreground && self.view.page == Page::Running && self.view.layers.is_empty() && self.streaming() && self.prefs.stats.mode.on()
    }
    /// The seconds the far side's idle countdown has left, while its warning is up over a stream being watched. Counted from the
    /// model's clock, so the prompt needs no event per second.
    pub fn idle_left(&self) -> Option<u32> {
        let warning = self.view.idle?;
        (self.foreground && self.view.page == Page::Running && self.streaming())
            .then(|| warning.ends_ms.saturating_sub(self.now_ms).div_ceil(1000) as u32)
    }
    pub fn elapsed(&self) -> u32 {
        self.session.as_ref().and_then(|s| s.played(self.now_ms)).unwrap_or(0)
    }
    /// The end-session sheet's title, explanation and confirming button, in the words of the engine's kind.
    pub fn end_copy(&self) -> (&'static str, String, &'static str) {
        let engine = self.session.as_ref().map(|s| &s.source.engine);
        let kind = engine.and_then(|id| self.integration(id)).map(|e| e.manifest.kind);
        let place = match (kind, engine) {
            (Some(EngineKind::Host), _) => "on the PC".to_string(),
            (Some(EngineKind::Cloud), Some(id)) => format!("on {}", self.engine_name(id)),
            _ => String::new(),
        };
        if self.can_end_game() {
            let body = if place.is_empty() { "The game will close.".to_string() } else { format!("The game will close {place}.") };
            ("End this session?", format!("{body} Unsaved progress may be lost."), "End session")
        } else {
            let body = if place.is_empty() { "The game can keep running.".to_string() } else { format!("The game can keep running {place}.") };
            ("Leave the stream?", format!("The stream will close. {body}"), "Leave stream")
        }
    }
    /// A key pressed while the far side's idle countdown runs. The remote and a keyboard never reach the stream, and an overlay
    /// pad key is the menu's, so the engine sends the far side one input for that press: it is the activity the countdown asks for.
    /// The prompt clears when the far side's own clear arrives, not on the press. At most one a second, whatever is held down.
    pub(super) fn idle_activity(&mut self, out: &mut Vec<Command>) {
        let (Some(warning), Some(s)) = (self.view.idle.as_mut(), self.session.as_ref()) else { return };
        if warning.sent_ms.is_some_and(|at| self.now_ms < at + 1000) {
            return;
        }
        warning.sent_ms = Some(self.now_ms);
        Self::send(out, &s.source.engine, Request::Activity { session: s.id });
    }
    pub(super) fn set_stats(&mut self, mode: StatsMode, out: &mut Vec<Command>) {
        if mode == self.prefs.stats.mode { return; }
        let unix_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_millis() as u64);
        self.prefs.stats = Overlay { mode, changed: unix_ms.max(self.prefs.stats.changed.saturating_add(1)) };
        if !mode.on() { self.telemetry = Telemetry::default(); } else { self.telemetry.rest = Default::default(); }
        self.save_prefs(out);
        self.stats_request(out);
    }
    pub(super) fn start(&mut self, game: GameId, source: SourceKey, remember: bool, out: &mut Vec<Command>) {
        if self.session_active() {
            return;
        }
        let presentation = self.integration(&source.engine).and_then(|e| e.descriptor.as_ref()).map_or(Presentation::Feeder, |d| d.capabilities.presentation);
        let (session, effects) = Session::start(SessionId(self.next_session), game, source, presentation, remember, self.now_ms);
        self.next_session = self.next_session.wrapping_add(1);
        self.session_start = self.unix_s;
        self.session = Some(session);
        self.ended_by = None;
        self.view.selected = Some(game);
        self.view.notice = None;
        self.view.stream = None;
        self.view.stats = Stats::default();
        self.telemetry = Telemetry::default();
        self.queue = QueueWatch::default();
        self.view.page = Page::Launch;
        self.view.clear_layers();
        self.view.focus = Focus::None;
        self.session_effects(effects, out);
    }
    pub(super) fn session_input(&mut self, input: session::Input, out: &mut Vec<Command>) {
        if let Some(session) = &mut self.session {
            let cause = match &input {
                session::Input::Cancel | session::Input::Stop { .. } => if self.closing { Cause::Close } else { Cause::Owner },
                session::Input::Background => Cause::Background,
                session::Input::EngineGone => Cause::EngineExit,
                session::Input::Engine(_) => Cause::Engine,
                session::Input::Tick => Cause::Timeout,
            };
            let effects = session.step(self.now_ms, input);
            if self.ended_by.is_none() && matches!(session.phase(), Phase::Stopping | Phase::Done(_)) { self.ended_by = Some(cause); }
            self.queue.observe(session.phase());
            self.session_effects(effects, out);
        }
    }
    fn checkpoint(&self, seconds: u32) -> Option<OpenSession> {
        let s = self.session.as_ref()?;
        Some(OpenSession { profile: self.state.active.clone(), played: Played { game: s.game, source: s.source.clone(), start: self.session_start, seconds } })
    }
    fn session_effects(&mut self, effects: Vec<session::Effect>, out: &mut Vec<Command>) {
        for effect in effects {
            let Some(s) = &self.session else {
                break;
            };
            match effect {
                session::Effect::Launch => {
                    Self::send(out, &s.source.engine, Request::Launch { session: s.id, target: s.source.target.clone(), game: s.source.id.clone() })
                }
                session::Effect::Stop { end_game } => {
                    Self::send(out, &s.source.engine, Request::Stop { session: s.id, end_game });
                    self.view.page = Page::Launch;
                    self.view.clear_layers();
                }
                session::Effect::LowLatency(on) => out.push(Command::LowLatency(on)),
                session::Effect::Started { pin } => {
                    self.state.profile_mut().user.started(s.game, &s.source, pin);
                    self.session_start = self.unix_s;
                    self.view.page = Page::Running;
                    self.view.clear_layers();
                    self.strip.show(self.now_ms);
                    self.view.auth = None;
                    out.push(Command::SaveState);
                    self.stats_request(out);
                    if let Some(checkpoint) = self.checkpoint(0) {
                        out.push(Command::Checkpoint(checkpoint));
                    }
                }
                session::Effect::Checkpoint { seconds } => {
                    if let Some(checkpoint) = self.checkpoint(seconds) {
                        out.push(Command::Checkpoint(checkpoint));
                    }
                }
                session::Effect::EndEngine => out.push(Command::KillEngine(s.source.engine.clone())),
                session::Effect::Finished { played, outcome } => {
                    if let Some(checkpoint) = played.and_then(|seconds| self.checkpoint(seconds)) {
                        self.state.record(checkpoint);
                        out.push(Command::SaveState);
                    }
                    self.view.page = Page::Game;
                    // The in-stream menu cannot be gone back to once its stream has ended.
                    self.back.retain(|way| way.page != Page::Running);
                    self.view.clear_layers();
                    self.view.focus = Focus::HubAction(0);
                    self.view.hub_tab = 0;
                    self.view.idle = None;
                    self.view.notice = match outcome {
                        session::Outcome::Failed { message, .. } => Some(message),
                        session::Outcome::Ended(ipc::EndReason::Remote) => Some("The game ended.".into()),
                        // Calm and without a fault: the far side's own policy ended it, and playing again is one press away.
                        session::Outcome::Ended(ipc::EndReason::Idle) => Some(format!("{} ended the session after inactivity.", self.engine_name(&s.source.engine))),
                        _ => None,
                    };
                    out.push(Command::ClearCheckpoint);
                    self.rebuild_rows();
                    if self.closing {
                        out.push(Command::Exit);
                    } else {
                        self.refresh_accounts(false, out);
                    }
                }
            }
        }
    }
    pub(super) fn open_overlay(&mut self, out: &mut Vec<Command>) {
        if let Some(s) = self.session.as_ref().filter(|s| *s.phase() == Phase::Running) {
            let (engine, id) = (s.source.engine.clone(), s.id);
            self.strip.show(self.now_ms);
            (self.view.page, self.view.pane, self.view.search) = (Page::Running, None, false);
            self.view.clear_layers();
            self.view.focus = Focus::None;
            self.push_layer(Layer::StreamMenu, Focus::Overlay(0));
            Self::send(out, &engine, Request::Route { session: id, input: ipc::InputRoute::App });
            self.stats_request(out);
        }
    }
    /// The menu closes and the pad goes back to the game first, so Steam's overlay opens over the game and gets the pad.
    pub(super) fn open_steam_overlay(&mut self, out: &mut Vec<Command>) {
        self.resume(out);
        out.push(Command::SteamOverlay(self.session.as_ref().map(|s| (s.source.engine.clone(), s.id))));
    }
    pub(super) fn resume(&mut self, out: &mut Vec<Command>) {
        if let Some(s) = &self.session {
            self.view.page = Page::Running;
            self.view.clear_layers();
            self.view.focus = Focus::None;
            Self::send(out, &s.source.engine, Request::Route { session: s.id, input: ipc::InputRoute::Game });
            self.stats_request(out);
        }
    }
    /// The in-stream menu's buttons, in order.
    pub fn overlay_actions(&self) -> impl Iterator<Item = OverlayAction> + use<> {
        use OverlayAction::*;
        let steam = self.steam_overlay();
        [Resume, Settings, Stats, SteamOverlay, EndSession].into_iter().filter(move |a| steam || *a != SteamOverlay)
    }
    /// The running game is a Steam copy and its engine can open Steam's overlay over it.
    pub(super) fn steam_overlay(&self) -> bool {
        self.session.as_ref().is_some_and(|s| {
            self.integration(&s.source.engine).and_then(|e| e.descriptor.as_ref()).is_some_and(|d| d.capabilities.has(Feature::SteamOverlay))
                && self.game(s.game).and_then(|g| g.source(&s.source)).is_some_and(|source| source.entry.store.as_deref() == Some("steam"))
        })
    }
    pub fn can_end_game(&self) -> bool {
        self.session
            .as_ref()
            .and_then(|s| self.integration(&s.source.engine))
            .and_then(|e| e.descriptor.as_ref())
            .is_some_and(|d| d.capabilities.has(Feature::EndGame))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::tests::*;
    #[test]
    fn queue_progress_follows_the_line_forward_and_starts_over_with_each_launch() {
        let mut m = launching("Game", false);
        let line = |position| SessionState::Queue { position, eta_s: None };
        assert_eq!(m.queue_progress(), None, "nothing is known before the queue says where the launch stands");
        let shares: Vec<_> = [Some(12), Some(9), Some(10), Some(6), None, Some(1)]
            .into_iter()
            .map(|position| {
                report(&mut m, line(position));
                m.queue_progress()
            })
            .collect();
        assert_eq!(shares, [Some(0.0), Some(0.25), Some(0.25), Some(0.5), None, Some(11.0 / 12.0)], "forward only, and never the whole way while still waiting");
        report(&mut m, SessionState::Starting);
        assert_eq!(m.queue_progress(), None, "past the queue only motion is honest");
        for (first, share) in [(1, None), (4, Some(0.0))] {
            m.session = None;
            m.start(GameId(1), "gfn|main|1".parse().unwrap(), false, &mut Vec::new());
            report(&mut m, line(Some(first)));
            assert_eq!(m.queue_progress(), share, "each launch counts from where its own queue began");
        }
    }
    #[test]
    fn cancel_on_the_launch_screen_is_a_direction_away_and_never_preselected() {
        let phase = |m: &Model| m.session().map(|s| s.phase().clone());
        let mut m = launching("Game", false);
        assert_eq!((m.view.focus, m.can_cancel()), (Focus::None, true));
        key(&mut m, Key::Ok);
        assert_eq!(phase(&m), Some(Phase::Checking), "an OK pressed for something else does not cancel");
        key(&mut m, Key::Down);
        assert_eq!(m.view.focus, Focus::Cancel);
        key(&mut m, Key::Ok);
        assert_eq!(phase(&m), Some(Phase::Stopping));
        assert!(!m.can_cancel() && m.navigation.is_empty() && m.view.focus == Focus::None, "a stop already under way cannot be cancelled again");
        let mut m = launching("Game", false);
        key(&mut m, Key::Back);
        assert_eq!(phase(&m), Some(Phase::Stopping), "Back still cancels");
    }
    #[test]
    fn the_first_input_that_ends_a_session_is_the_cause_diagnostics_reports() {
        let ended_by = |m: &Model| m.observed().and_then(|seen| seen.ended_by);
        let mut m = launching("Game", false);
        assert_eq!(ended_by(&m), None);
        key(&mut m, Key::Back);
        assert_eq!(ended_by(&m), Some("owner"));
        report(&mut m, SessionState::Ended { reason: ipc::EndReason::Stopped });
        assert_eq!(ended_by(&m), Some("owner"), "what the engine reports afterwards does not change the cause");
        for word in ["engine", "background", "app_close", "engine_exit"] {
            let mut m = launching("Game", false);
            match word {
                "engine" => report(&mut m, SessionState::Ended { reason: ipc::EndReason::Remote }),
                "background" => drop(m.step(Input::Foreground(false))),
                "app_close" => drop(m.step(Input::Close)),
                _ => drop(m.step(Input::EngineGone(EngineId::new("gfn").unwrap()))),
            }
            assert_eq!(ended_by(&m), Some(word));
        }
    }
    #[test]
    fn sheets_return_focus_and_destructive_confirmation_starts_safe() {
        let mut m = launching("Game", false);
        report(&mut m, SessionState::Running { stream: None });
        m.step(Input::Overlay);
        m.view.focus = Focus::Overlay(3);
        key(&mut m, Key::Ok);
        assert_eq!(m.view.top(), Some(Layer::EndSession));
        assert_eq!(m.view.focus, Focus::SheetAction(0));
        key(&mut m, Key::Back);
        assert_eq!(m.view.focus, Focus::Overlay(3));
    }
    #[test]
    fn steam_overlay_is_offered_over_steam_games_and_presses_after_the_menu_closes() {
        use OverlayAction::*;
        let mut m = launching("Game", false);
        report(&mut m, SessionState::Running { stream: None });
        m.step(Input::Overlay);
        describe(&mut m, "gfn", vec![], &[Feature::EndGame, Feature::SteamOverlay]);
        assert_eq!(m.overlay_actions().collect::<Vec<_>>(), [Resume, Settings, Stats, EndSession], "not a Steam copy");
        m.games[0].sources[0].entry.store = Some("steam".into());
        assert_eq!(m.overlay_actions().collect::<Vec<_>>(), [Resume, Settings, Stats, SteamOverlay, EndSession]);
        m.view.focus = Focus::Overlay(3);
        let out = key(&mut m, Key::Ok);
        let session = m.session().unwrap().id;
        assert_eq!(m.view.page, Page::Running);
        assert_eq!(sent(&out).first(), Some(&&Request::Route { session, input: ipc::InputRoute::Game }));
        let overlay = Command::SteamOverlay(Some((EngineId::new("gfn").unwrap(), session)));
        assert_eq!(out.last(), Some(&overlay), "the request follows the menu closing");
        m.step(Input::Overlay);
        m.view.focus = Focus::Overlay(0);
        assert_eq!(key(&mut m, Key::Alt).last(), Some(&overlay), "X opens it from any item");
        assert_eq!(m.view.page, Page::Running);
    }
    #[test]
    fn x_opens_nothing_over_a_game_that_is_not_a_steam_copy() {
        let mut m = launching("Game", false);
        report(&mut m, SessionState::Running { stream: None });
        m.step(Input::Overlay);
        describe(&mut m, "gfn", vec![], &[Feature::EndGame, Feature::SteamOverlay]);
        assert!(key(&mut m, Key::Alt).is_empty());
        assert!(m.stream_menu());
    }
    #[test]
    fn a_stream_page_never_outlives_its_session() {
        let mut m = launching("Game", false);
        report(&mut m, SessionState::Running { stream: None });
        m.step(Input::Overlay);
        m.view.focus = Focus::Overlay(1);
        key(&mut m, Key::Ok);
        key(&mut m, Key::Back);
        assert_eq!((m.stream_menu(), m.view.focus), (true, Focus::Overlay(1)), "Settings opened from the in-stream menu returns to it");
        key(&mut m, Key::Ok);
        report(&mut m, SessionState::Ended { reason: ipc::EndReason::Remote });
        assert_eq!(m.view.page, Page::Game);
        key(&mut m, Key::Back);
        assert_eq!(m.view.page, Page::Home, "Back after the stream ended skips its menu");
        m.view.page = Page::Running;
        m.step(Input::Foreground(true));
        assert_eq!((m.view.page, m.view.focus), (Page::Game, Focus::HubAction(0)), "a relaunch onto a stream page without a stream shows the game");
        assert!(m.wants_frames(), "and the shell draws it");
    }
    #[test]
    fn ending_a_cloud_session_names_the_service_not_a_pc() {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud), manifest("gamestream", "Sunshine", EngineKind::Host)]);
        describe(&mut m, "gfn", vec![], &[Feature::EndGame]);
        describe(&mut m, "gamestream", vec![], &[Feature::EndGame]);
        let key = |engine: &str| SourceKey { engine: EngineId::new(engine).unwrap(), target: TargetId::new("main").unwrap(), id: lumagame_core::ids::ProviderGameId::new("1").unwrap() };
        m.session = Some(Session::start(SessionId(1), GameId(1), key("gfn"), Presentation::Feeder, false, 0).0);
        assert!(!m.wants_pads(), "the menu lets go of the pads as soon as a launch begins");
        assert_eq!(m.end_copy().1, "The game will close on GeForce NOW. Unsaved progress may be lost.");
        m.session = Some(Session::start(SessionId(2), GameId(1), key("gamestream"), Presentation::Feeder, false, 0).0);
        assert_eq!(m.end_copy().1, "The game will close on the PC. Unsaved progress may be lost.");
    }
    #[test]
    fn a_game_that_ends_on_the_host_returns_to_its_page_with_a_notice() {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("gfn", "GeForce NOW", EngineKind::Cloud)]);
        describe(&mut m, "gfn", vec![], &[]);
        m.session = Some(Session::start(SessionId(1), GameId(1), "gfn|main|1".parse().unwrap(), Presentation::Feeder, false, 0).0);
        for state in [SessionState::Running { stream: None }, SessionState::Ended { reason: ipc::EndReason::Remote }] {
            let event = ipc::Event::Session { session: SessionId(1), state };
            m.step(Input::Engine { engine: EngineId::new("gfn").unwrap(), message: Box::new(FromEngine { re: None, event }) });
        }
        assert_eq!((m.view.page, m.view.top(), m.view.notice.as_deref()), (Page::Game, None, Some("The game ended.")));
        assert!(!m.session_active(), "nothing is left for the end-stream prompt");
    }
}
