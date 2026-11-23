//! A session from launch to its end: what the start screen says at each step, the strip and the drawers over the running stream, and
//! ending a session.
use super::facts::{Fact, Reason, why};
use super::nav::{AutoHide, Hints, Slice};
use super::*;
use lumagame_core::session::Outcome;
use ui::components::Icon;

/// The start screen's logo and line rest at 40 % light after this long without a key; the ring keeps turning at full light.
pub const DIM_AFTER_MS: u64 = 60_000;
/// The ring is `RING_FRAMES` images, each shown for `RING_STEP_MS`: one turn in 1.2 s.
pub const RING_FRAMES: u8 = 16;
const RING_STEP_MS: u64 = 75;
/// The drawers and the End prompt over the stream close after this long without a key.
const LAYER_QUIET_MS: u64 = 30_000;
/// How long after the strip's time is up it still counts as on screen, so that it can leave with its animation.
const STRIP_EXIT_MS: u64 = 250;

/// The far side's idle countdown while it runs: when it ends by the model's clock, and when a press last told the engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IdleWarning {
    pub ends_ms: u64,
    pub(super) sent_ms: Option<u64>,
}
/// What moved the session towards its end, for diagnostics. It is read from the input that did it, so an engine's own wording never has to be parsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause { Owner, Close, Background, Engine, EngineExit, Timeout }
impl Cause {
    pub fn word(self) -> &'static str {
        match self { Cause::Owner => "owner", Cause::Close => "app_close", Cause::Background => "background", Cause::Engine => "engine", Cause::EngineExit => "engine_exit", Cause::Timeout => "timeout" }
    }
}
/// What the start screen and the strip keep between steps.
#[derive(Clone, Copy, Debug)]
pub struct Run {
    /// When the launch began: the ring's turn and the dimming count from it.
    pub(super) began_ms: u64,
    /// The PC was asleep when Play was pressed and its engine can wake it.
    pub(super) waking: bool,
    /// The pad is routed to the app, not the game, while the strip or a layer over the stream is up.
    pub(super) held: bool,
    /// One quiet timer for the drawers and the End prompt: any key refreshes it.
    pub(super) quiet: AutoHide,
}
impl Default for Run {
    fn default() -> Self {
        Self { began_ms: 0, waking: false, held: false, quiet: AutoHide::new(LAYER_QUIET_MS) }
    }
}
/// What the start screen says: the step the launch is at and, under it, where it happens and how long it may wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub headline: String,
    pub caption: Option<String>,
}
/// The strip's items in order; `Focus::Strip(i)` is the `i`th of [`StripItem::ALL`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripItem {
    Resume,
    Picture,
    Controller,
    Achievements,
    End,
}
impl StripItem {
    pub const ALL: [Self; 5] = [Self::Resume, Self::Picture, Self::Controller, Self::Achievements, Self::End];
    pub fn label(self) -> &'static str {
        match self {
            Self::Resume => "Resume",
            Self::Picture => "Picture",
            Self::Controller => "Controller",
            Self::Achievements => "Achievements",
            Self::End => "End",
        }
    }
}
/// One row of the Picture drawer: a stepper with its value, or a switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PictureRow {
    pub label: &'static str,
    pub icon: Icon,
    pub switch: bool,
    pub value: String,
    pub on: bool,
}
/// One line of the achievements drawer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AchievementRow {
    pub name: String,
    pub note: String,
    pub unlocked: bool,
}
/// What the achievements drawer shows: a list, or the one line that says why there is none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Achieved {
    Words(String),
    List { summary: String, rows: Vec<AchievementRow> },
}
/// How long a queue may take, in the words a caption uses.
fn wait(secs: u64) -> String {
    if secs < 60 { "less than a minute".into() } else { format!("about {} min", (secs + 30) / 60) }
}

pub struct Rules;
impl Slice for Rules {
    fn rows(m: &Model, layer: Option<Layer>) -> Vec<Vec<Focus>> {
        match (layer, m.view.page) {
            (Some(Layer::Picture), _) => (0..PICTURE.len()).map(|i| vec![Focus::SheetRow(i)]).collect(),
            (Some(Layer::Achievements), _) => match m.achievement_list() {
                Achieved::List { rows, .. } => (0..rows.len()).map(|i| vec![Focus::SheetRow(i)]).collect(),
                Achieved::Words(_) => Vec::new(),
            },
            (Some(Layer::EndConfirm), _) => vec![vec![Focus::SheetAction(0), Focus::SheetAction(1)]],
            (None, Page::Running) if m.strip_visible() => vec![(0..StripItem::ALL.len()).map(Focus::Strip).collect()],
            _ => Vec::new(),
        }
    }
    fn activate(m: &mut Model, f: Focus, out: &mut Vec<Command>) {
        match (f, m.view.top()) {
            (Focus::Strip(i), None) => {
                if let Some(item) = StripItem::ALL.get(i) {
                    m.strip_activate(*item, out);
                }
            }
            (Focus::SheetAction(0), Some(Layer::EndConfirm)) => m.close_stream_layer(out),
            (Focus::SheetAction(_), Some(Layer::EndConfirm)) => {
                m.view.clear_layers();
                m.session_input(session::Input::Stop { end_game: m.can_end_game() }, out);
            }
            (Focus::SheetRow(i), Some(Layer::Picture)) => {
                if let Some(setting) = PICTURE.get(i) {
                    setting.adjust(m, true, out);
                }
            }
            _ => {}
        }
    }
    fn adjust(m: &mut Model, f: Focus, forward: bool, out: &mut Vec<Command>) -> bool {
        match (f, m.view.top()) {
            (Focus::SheetRow(i), Some(Layer::Picture)) => {
                if let Some(setting) = PICTURE.get(i) {
                    setting.adjust(m, forward, out);
                }
                true
            }
            _ => false,
        }
    }
    fn subject(m: &Model) -> Option<GameId> {
        m.session.as_ref().map(|s| s.game)
    }
    fn hints(m: &Model) -> Hints {
        match (m.view.page, m.view.top()) {
            (Page::Launch, _) if m.can_cancel() => Hints(vec![(Key::Back, "Cancel")]),
            (Page::Running, None) if m.strip_visible() && m.steam_overlay() => Hints(vec![(Key::Alt, "Steam overlay")]),
            (Page::Running, Some(Layer::EndConfirm)) => Hints(vec![(Key::Back, "Keep playing")]),
            (Page::Running, Some(_)) => Hints(vec![(Key::Back, "Back")]),
            _ => Hints::default(),
        }
    }
}
/// The settings the Picture drawer changes, in its order.
const PICTURE: [QuickSetting; 3] = [QuickSetting::Stream, QuickSetting::Hdr, QuickSetting::Stats];

impl Model {
    /// Why the current session of `id` failed, in the engine's words.
    pub fn last_failure(&self, id: GameId) -> Option<&str> {
        let session = self.session.as_ref().filter(|s| s.game == id)?;
        match session.phase() {
            Phase::Done(Outcome::Failed { message, .. }) => Some(message),
            _ => None,
        }
    }
    /// Whether the launch can still be called off: a stop that has begun cannot be.
    pub fn can_cancel(&self) -> bool {
        self.session.as_ref().is_some_and(|s| matches!(s.phase(), Phase::Checking | Phase::SignIn | Phase::Queue { .. } | Phase::Starting))
    }
    /// The step the launch is at, in the words of what its engine really does: only an engine with a queue ever says "In queue".
    pub fn start_step(&self) -> Option<Step> {
        let s = self.session.as_ref()?;
        let engine = self.integration(&s.source.engine);
        let kind = engine.map(|e| e.manifest.kind);
        let has = |feature: Feature| engine.and_then(|e| e.descriptor.as_ref()).is_some_and(|d| d.capabilities.has(feature));
        let name = self.engine_name(&s.source.engine).to_string();
        let target = engine.and_then(|e| e.targets.iter().find(|t| t.id == s.source.target)).map(|t| t.name.clone());
        let step = |headline: &str, caption: Option<String>| Some(Step { headline: headline.into(), caption });
        match (s.phase(), kind) {
            (Phase::Stopping | Phase::Done(_), _) => step("Closing the session", None),
            (Phase::Queue { position, eta_s }, Some(EngineKind::Cloud)) if has(Feature::Queue) => {
                let headline = match position.map(u64::from) {
                    Some(n) if n > 1 => format!("In queue, {} ahead", n - 1),
                    Some(_) => "Next in line".to_string(),
                    None => "In queue".to_string(),
                };
                let caption = match eta_s.map(u64::from) {
                    Some(secs) => format!("{name}, {}", wait(secs)),
                    None => name,
                };
                step(&headline, Some(caption))
            }
            (Phase::SignIn, Some(EngineKind::Cloud)) => step("Signing in", Some(name)),
            (Phase::Checking, Some(EngineKind::Cloud)) => step("Connecting", Some(name)),
            (Phase::Checking | Phase::SignIn, Some(EngineKind::Host)) => {
                step(if self.run.waking { "Waking the PC" } else { "Connecting to the PC" }, target.or(Some(name)))
            }
            (_, Some(EngineKind::Host)) => step("Starting your game", target.or(Some(name))),
            (_, Some(EngineKind::Cloud)) => step("Starting your game", Some(name)),
            (_, Some(EngineKind::Native | EngineKind::Emulator)) => step("Starting", Some("On this TV".into())),
            _ => step("Starting", Some(name)),
        }
    }
    /// Which of the ring's images shows now. Reduced motion keeps the first.
    pub fn ring_frame(&self) -> u8 {
        if self.prefs.reduce_motion {
            return 0;
        }
        (self.ring_step(self.now_ms) % u64::from(RING_FRAMES)) as u8
    }
    fn ring_step(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.run.began_ms) / RING_STEP_MS
    }
    fn dimmed_at(&self, now_ms: u64) -> bool {
        self.view.page == Page::Launch && now_ms.saturating_sub(self.run.began_ms.max(self.last_key_ms)) >= DIM_AFTER_MS
    }
    /// The start screen has gone a minute without a key: its logo and line rest dimmed.
    pub fn start_dim(&self) -> bool {
        self.dimmed_at(self.now_ms)
    }
    /// The Picture drawer's rows.
    pub fn picture_rows(&self) -> Vec<PictureRow> {
        PICTURE.iter().map(|s| PictureRow { label: s.label(), icon: s.icon(), switch: s.is_switch(), value: s.value(self), on: s.on(self) }).collect()
    }
    /// The achievements drawer's list, as the game page's facts know it.
    pub fn achievement_list(&self) -> Achieved {
        let Some(game) = self.session.as_ref().map(|s| s.game) else {
            return Achieved::Words(why(Reason::Empty).into());
        };
        match self.facts(game).activity.achievements {
            Fact::Present(a) => {
                let summary = match a.total {
                    Some(total) => format!("{} of {total} unlocked", a.unlocked),
                    None => format!("{} unlocked", a.unlocked),
                };
                let note = |rarity: Option<u16>, lead: &str| match rarity {
                    Some(r) => format!("{lead} \u{b7} {}.{} % of players", r / 10, r % 10),
                    None => lead.to_string(),
                };
                let rows = a
                    .latest
                    .iter()
                    .map(|x| AchievementRow { name: x.name.to_string(), note: note(x.rarity, "Unlocked"), unlocked: true })
                    .chain(a.next.iter().map(|x| AchievementRow { name: x.name.to_string(), note: note(x.rarity, "Locked"), unlocked: false }))
                    .collect();
                Achieved::List { summary, rows }
            }
            Fact::Loading => Achieved::Words("Loading achievements".into()),
            Fact::Absent(reason) => Achieved::Words(why(reason).into()),
            Fact::SignIn { expired } => Achieved::Words(if expired { "Sign in again to see achievements" } else { "Sign in to see achievements" }.into()),
        }
    }
    /// The figures of the stream the pill over it shows: the picture, the frame rate and the round trip, where they are known.
    pub fn figures(&self) -> Vec<String> {
        let mut figures = Vec::new();
        if let Some(stream) = &self.view.stream {
            figures.push(if stream.height >= 2160 { "4K".to_string() } else { format!("{}p", stream.height) });
            figures.push(format!("{} fps", stream.fps));
        }
        if let Some(rtt) = self.view.stats.rtt_ms {
            figures.push(format!("{} ms", rtt.round() as u32));
        }
        figures
    }
    /// What the running stream was asked for, which the HUD's diagnosis weighs the readings against.
    pub(super) fn asked(&self) -> crate::game::stats::Asked {
        let stream = self.view.stream.as_ref();
        crate::game::stats::Asked { fps: stream.map_or(self.prefs.fps, |s| s.fps), cap_mbps: self.stream_bitrate_mbps(), sdr_reason: stream.and_then(|s| s.sdr_reason.clone()) }
    }
    pub fn hud_visible(&self) -> bool {
        self.foreground && self.view.page == Page::Running && self.view.layers.is_empty() && self.streaming() && self.prefs.stats.mode.on()
    }
    /// Nothing of the app is drawn over the stream: no strip (it keeps its exit animation a moment), no drawer.
    pub fn stream_bare(&self) -> bool {
        self.view.page == Page::Running && self.view.layers.is_empty() && !self.strip.visible(self.now_ms.saturating_sub(STRIP_EXIT_MS))
    }
    /// The strip or a layer is up over the stream, which is when the engine reports its stats for the figures.
    pub(super) fn over_stream(&self) -> bool {
        self.view.page == Page::Running && (!self.view.layers.is_empty() || self.strip.visible(self.now_ms))
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
    /// The engine can wake the PC and the PC is asleep.
    fn wakes(&self, source: &SourceKey) -> bool {
        self.integration(&source.engine).is_some_and(|e| {
            e.descriptor.as_ref().is_some_and(|d| d.capabilities.has(Feature::Wake))
                && e.targets.iter().any(|t| t.id == source.target && matches!(t.reach, ipc::Reach::Asleep))
        })
    }
    pub(super) fn start(&mut self, game: GameId, source: SourceKey, remember: bool, out: &mut Vec<Command>) {
        if self.session_active() {
            return;
        }
        let presentation = self.integration(&source.engine).and_then(|e| e.descriptor.as_ref()).map_or(Presentation::Feeder, |d| d.capabilities.presentation);
        self.run = Run { began_ms: self.now_ms, waking: self.wakes(&source), ..Run::default() };
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
                    self.run.held = false;
                }
                session::Effect::LowLatency(on) => out.push(Command::LowLatency(on)),
                session::Effect::Started { pin } => {
                    self.state.profile_mut().user.started(s.game, &s.source, pin);
                    self.session_start = self.unix_s;
                    self.view.page = Page::Running;
                    self.view.clear_layers();
                    self.view.focus = Focus::None;
                    // The first seconds show the strip and the figures; the pad stays with the game until a key asks for the menu.
                    self.strip.show(self.now_ms);
                    self.run.held = false;
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
                    self.view.clear_layers();
                    self.view.focus = Focus::HubAction(0);
                    self.view.hub_tab = 0;
                    self.view.idle = None;
                    self.run.held = false;
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
    /// What the launch screen and the strip keep on the model's clock between `before_ms` and now: the ring's next image, the dimming,
    /// the strip's end (which hands the pad back) and the drawers' quiet time. True when the screen changes.
    pub(super) fn tick_launch(&mut self, before_ms: u64, out: &mut Vec<Command>) -> bool {
        let now = self.now_ms;
        match self.view.page {
            Page::Launch => (!self.prefs.reduce_motion && self.ring_step(before_ms) != self.ring_step(now)) | (self.dimmed_at(before_ms) != self.dimmed_at(now)),
            Page::Running if self.view.layers.is_empty() => {
                let gone = self.strip.visible(before_ms) && !self.strip.visible(now);
                if gone {
                    self.give_pad_back(out);
                    self.stats_request(out);
                }
                gone
            }
            Page::Running if !self.run.quiet.visible(now) => {
                self.resume(out);
                true
            }
            _ => false,
        }
    }
    /// Milliseconds until the launch screen or the strip next needs a pass of the loop: the ring's next image, the dimming, the
    /// strip leaving and its exit animation ending, the drawers' quiet time running out.
    pub(super) fn launch_wake(&self, now_ms: u64) -> Option<u64> {
        match self.view.page {
            Page::Launch => {
                let ring = (!self.prefs.reduce_motion).then(|| RING_STEP_MS - now_ms.saturating_sub(self.run.began_ms) % RING_STEP_MS);
                let dim = (!self.dimmed_at(now_ms)).then(|| (self.run.began_ms.max(self.last_key_ms) + DIM_AFTER_MS).saturating_sub(now_ms));
                [ring, dim].into_iter().flatten().min()
            }
            Page::Running if self.view.layers.is_empty() => self.strip.left(now_ms).or_else(|| self.strip.left(now_ms.saturating_sub(STRIP_EXIT_MS))),
            Page::Running => self.run.quiet.left(now_ms),
            _ => None,
        }
    }
    /// Sends the pad to the app while the strip or a layer is up, so a controller can work them.
    fn take_pad(&mut self, out: &mut Vec<Command>) {
        if let (false, Some(s)) = (self.run.held, &self.session) {
            Self::send(out, &s.source.engine, Request::Route { session: s.id, input: ipc::InputRoute::App });
            self.run.held = true;
        }
    }
    /// Gives the pad back to the game, if the app had it.
    fn give_pad_back(&mut self, out: &mut Vec<Command>) {
        if let (true, Some(s)) = (self.run.held, &self.session) {
            Self::send(out, &s.source.engine, Request::Route { session: s.id, input: ipc::InputRoute::Game });
            self.run.held = false;
        }
    }
    /// The strip comes up on the Guide button, or on the engine's say-so that the pad's overlay combination was pressed.
    pub(super) fn open_strip(&mut self, out: &mut Vec<Command>) -> bool {
        if !self.session.as_ref().is_some_and(|s| *s.phase() == Phase::Running) {
            return false;
        }
        (self.view.page, self.view.pane, self.view.search) = (Page::Running, None, false);
        self.view.clear_layers();
        self.strip.show(self.now_ms);
        self.view.focus = Focus::Strip(0);
        self.take_pad(out);
        self.stats_request(out);
        true
    }
    /// The strip and every layer over the stream close, and the pad goes back to the game.
    pub(super) fn resume(&mut self, out: &mut Vec<Command>) {
        self.view.clear_layers();
        self.strip.hide(self.now_ms);
        self.view.focus = Focus::None;
        self.give_pad_back(out);
        self.stats_request(out);
    }
    /// The strip closes and the pad goes back to the game first, so Steam's overlay opens over the game and gets the pad.
    pub(super) fn open_steam_overlay(&mut self, out: &mut Vec<Command>) {
        self.resume(out);
        out.push(Command::SteamOverlay(self.session.as_ref().map(|s| (s.source.engine.clone(), s.id))));
    }
    fn open_stream_layer(&mut self, layer: Layer, focus: Focus, out: &mut Vec<Command>) {
        self.take_pad(out);
        self.run.quiet.show(self.now_ms);
        self.push_layer(layer, focus);
        self.stats_request(out);
    }
    /// A drawer or the prompt closes: the strip is back under it for the next move.
    pub(super) fn close_stream_layer(&mut self, out: &mut Vec<Command>) {
        self.pop_layer();
        if self.view.layers.is_empty() {
            self.strip.show(self.now_ms);
        }
        self.stats_request(out);
    }
    fn strip_activate(&mut self, item: StripItem, out: &mut Vec<Command>) {
        match item {
            StripItem::Resume => self.resume(out),
            StripItem::Picture => self.open_stream_layer(Layer::Picture, Focus::SheetRow(0), out),
            StripItem::Controller => out.push(Command::OpenControllerSettings),
            StripItem::Achievements => {
                if let Some(game) = self.session.as_ref().map(|s| s.game) {
                    self.request_owned(game, out);
                }
                self.open_stream_layer(Layer::Achievements, Focus::SheetRow(0), out);
            }
            StripItem::End => self.open_stream_layer(Layer::EndConfirm, Focus::SheetAction(0), out),
        }
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

// STAND-IN for `super::quick::QuickSetting`, which the quick-menu slice adds. Once that has landed, delete this module and write
// `use super::quick::QuickSetting;` in its place: the Picture drawer only calls what this offers.
use stand_in::QuickSetting;
mod stand_in {
    use super::*;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum QuickSetting {
        Stream,
        Hdr,
        Stats,
    }
    /// Width, height and frame rate of each stream the Stream row steps through.
    const STREAMS: [(u32, u32, u32); 8] =
        [(1280, 720, 60), (1280, 720, 120), (1920, 1080, 60), (1920, 1080, 120), (2560, 1440, 60), (2560, 1440, 120), (3840, 2160, 60), (3840, 2160, 120)];
    impl QuickSetting {
        pub fn label(self) -> &'static str {
            match self {
                Self::Stream => "Stream",
                Self::Hdr => "HDR",
                Self::Stats => "Stats while playing",
            }
        }
        pub fn icon(self) -> Icon {
            match self {
                Self::Stream => Icon::Sliders,
                Self::Hdr => Icon::Sun,
                Self::Stats => Icon::Info,
            }
        }
        pub fn is_switch(self) -> bool {
            !matches!(self, Self::Stream)
        }
        pub fn on(self, m: &Model) -> bool {
            match self {
                Self::Stream => false,
                Self::Hdr => m.prefs.hdr,
                Self::Stats => m.prefs.stats.mode.on(),
            }
        }
        pub fn value(self, m: &Model) -> String {
            match self {
                Self::Stream => {
                    let (h, fps) = (m.prefs.height, m.prefs.fps);
                    if h >= 2160 { format!("4K, {fps} fps") } else { format!("{h}p, {fps} fps") }
                }
                _ => if self.on(m) { "On" } else { "Off" }.to_string(),
            }
        }
        pub fn adjust(self, m: &mut Model, forward: bool, out: &mut Vec<Command>) {
            match self {
                Self::Stream => {
                    let at = STREAMS.iter().position(|(_, h, f)| *h == m.prefs.height && *f == m.prefs.fps).unwrap_or(0);
                    let next = if forward { (at + 1) % STREAMS.len() } else { (at + STREAMS.len() - 1) % STREAMS.len() };
                    let (w, h, fps) = STREAMS[next];
                    (m.prefs.width, m.prefs.height, m.prefs.fps) = (w, h, fps);
                    if fps > 60 && m.prefs.codec == Some(Codec::Av1) {
                        m.prefs.codec = None;
                    }
                    m.save_prefs(out);
                    m.push_common(out);
                }
                Self::Hdr => {
                    m.prefs.hdr = !m.prefs.hdr;
                    m.save_prefs(out);
                    m.push_common(out);
                }
                Self::Stats => m.set_stats(if m.prefs.stats.mode.on() { StatsMode::Off } else { StatsMode::Compact }, out),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::model::tests::*;
    use lumagame_core::ipc::SessionState;

    fn tick(m: &mut Model, now_ms: u64) -> Vec<Command> {
        m.step(Input::Tick { now_ms, unix_s: now_ms / 1000 })
    }
    /// Which way each route the pad got went: true for the app.
    fn routes(out: &[Command]) -> Vec<bool> {
        sent(out).into_iter().filter_map(|r| if let Request::Route { input, .. } = r { Some(matches!(input, ipc::InputRoute::App)) } else { None }).collect()
    }
    fn running() -> Model {
        let mut m = launching("Game", false);
        report(&mut m, SessionState::Running { stream: None });
        m
    }
    /// A launch on a fresh engine "eng" of `kind`, its target "main" in `reach`.
    fn launch_on(kind: EngineKind, features: &[Feature], reach: ipc::Reach) -> Model {
        let mut m = Model::new(State::default(), Listing::default(), vec![manifest("eng", "Engine", kind)]);
        describe(&mut m, "eng", vec![], features);
        let mut target = linked("main");
        target.reach = reach;
        m.integrations[0].targets = vec![target];
        m.start(GameId(1), "eng|main|1".parse().unwrap(), false, &mut Vec::new());
        m
    }
    fn says(m: &mut Model, engine: &str, state: SessionState) {
        let session = m.session().unwrap().id;
        engine_says(m, engine, ipc::Event::Session { session, state });
    }
    fn lines(m: &Model) -> (String, Option<String>) {
        let step = m.start_step().unwrap();
        (step.headline, step.caption)
    }
    fn in_queue(position: Option<u32>, eta_s: Option<u32>) -> SessionState {
        SessionState::Queue { position, eta_s }
    }

    #[test]
    fn the_start_says_what_its_engine_really_does() {
        let mut m = launching("Game", false);
        describe(&mut m, "gfn", vec![], &[Feature::EndGame, Feature::Queue]);
        assert_eq!(lines(&m), ("Connecting".into(), Some("GeForce NOW".into())));
        report(&mut m, SessionState::SignIn);
        assert_eq!(lines(&m).0, "Signing in");
        report(&mut m, in_queue(Some(15), Some(120)));
        assert_eq!(lines(&m), ("In queue, 14 ahead".into(), Some("GeForce NOW, about 2 min".into())));
        report(&mut m, in_queue(Some(1), Some(20)));
        assert_eq!(lines(&m), ("Next in line".into(), Some("GeForce NOW, less than a minute".into())));
        report(&mut m, in_queue(None, None));
        assert_eq!(lines(&m), ("In queue".into(), Some("GeForce NOW".into())));
        report(&mut m, SessionState::Starting);
        assert_eq!(lines(&m).0, "Starting your game");

        let mut m = launch_on(EngineKind::Host, &[Feature::Queue], ipc::Reach::Online);
        assert_eq!(lines(&m), ("Connecting to the PC".into(), Some("main".into())));
        says(&mut m, "eng", in_queue(Some(9), None));
        assert_eq!(lines(&m).0, "Starting your game", "a PC has no queue to be in");
        let m = launch_on(EngineKind::Host, &[Feature::Wake], ipc::Reach::Asleep);
        assert_eq!(lines(&m).0, "Waking the PC");
        let m = launch_on(EngineKind::Host, &[], ipc::Reach::Asleep);
        assert_eq!(lines(&m).0, "Connecting to the PC", "an engine that cannot wake it does not say it does");
        let m = launch_on(EngineKind::Native, &[], ipc::Reach::Online);
        assert_eq!(lines(&m), ("Starting".into(), Some("On this TV".into())));
    }
    #[test]
    fn a_cloud_engine_without_a_queue_never_says_in_queue() {
        let mut m = launching("Game", false);
        report(&mut m, in_queue(Some(15), Some(120)));
        assert_eq!(lines(&m).0, "Starting your game");
    }
    #[test]
    fn back_cancels_a_start_and_leaves_nothing_to_press() {
        let phase = |m: &Model| m.session().map(|s| s.phase().clone());
        let mut m = launching("Game", false);
        assert!(m.can_cancel() && m.navigation.is_empty() && m.view.focus == Focus::None);
        key(&mut m, Key::Ok);
        assert_eq!(phase(&m), Some(Phase::Checking), "an OK pressed for something else does not cancel");
        key(&mut m, Key::Back);
        assert_eq!(phase(&m), Some(Phase::Stopping));
        assert_eq!(lines(&m).0, "Closing the session");
        assert!(!m.can_cancel());
    }
    #[test]
    fn the_ring_turns_a_step_at_a_time_and_the_start_dims_after_a_minute_without_a_key() {
        let mut m = launching("Game", false);
        assert_eq!((m.ring_frame(), m.launch_wake(0)), (0, Some(RING_STEP_MS)));
        let seen: Vec<_> = (1..=u64::from(RING_FRAMES) + 1).map(|i| { tick(&mut m, i * RING_STEP_MS); m.ring_frame() }).collect();
        assert_eq!(seen[..3], [1, 2, 3]);
        assert_eq!(seen[usize::from(RING_FRAMES) - 1], 0, "a whole turn is back where it began");
        assert!(!m.start_dim());
        tick(&mut m, DIM_AFTER_MS - 1);
        assert!(!m.start_dim());
        assert!(!tick(&mut m, DIM_AFTER_MS).is_empty() || m.start_dim(), "the minute is up");
        assert!(m.start_dim());
        key(&mut m, Key::Ok);
        assert!(!m.start_dim(), "a key lights it again");
        m.prefs.reduce_motion = true;
        assert_eq!(m.ring_frame(), 0, "reduced motion keeps the ring still");
    }
    #[test]
    fn the_strip_leaves_after_five_seconds_and_the_pad_goes_back_to_the_game() {
        let mut m = running();
        assert!(m.strip_visible() && !m.stream_bare());
        assert!(routes(&tick(&mut m, 5_000)).is_empty(), "the intro never took the pad");
        assert!(!m.strip_visible() && !m.stream_bare(), "the strip keeps its exit animation a moment");
        tick(&mut m, 5_300);
        assert!(m.stream_bare() && !m.wants_frames());
        let out = m.step(Input::Overlay);
        assert_eq!((routes(&out), m.strip_visible(), m.view.focus), (vec![true], true, Focus::Strip(0)), "the pad's own combination opens it and takes the pad");
        assert_eq!(routes(&tick(&mut m, 10_400)), [false], "when it leaves the game has the pad again");
        assert!(routes(&tick(&mut m, 10_500)).is_empty());
    }
    #[test]
    fn the_first_key_over_a_bare_stream_only_shows_the_strip() {
        let mut m = running();
        tick(&mut m, 6_000);
        key(&mut m, Key::Ok);
        assert!(m.strip_visible() && m.session_active());
        assert_eq!(m.view.top(), None, "the OK did nothing else");
        key(&mut m, Key::Ok);
        assert!(!m.strip_visible(), "OK on Resume resumes");
    }
    #[test]
    fn the_picture_drawer_changes_the_stream_hdr_and_stats_and_back_returns_to_its_item() {
        let mut m = running();
        key(&mut m, Key::Right);
        assert_eq!(m.view.focus, Focus::Strip(1));
        key(&mut m, Key::Ok);
        assert_eq!((m.view.top(), m.view.focus), (Some(Layer::Picture), Focus::SheetRow(0)));
        assert_eq!(m.picture_rows().iter().map(|r| r.label).collect::<Vec<_>>(), ["Stream", "HDR", "Stats while playing"]);
        let fps = m.prefs.fps;
        key(&mut m, Key::Right);
        assert_ne!(m.prefs.fps, fps, "Right steps the stream");
        key(&mut m, Key::Down);
        let hdr = m.prefs.hdr;
        key(&mut m, Key::Right);
        assert_eq!(m.prefs.hdr, !hdr);
        key(&mut m, Key::Down);
        key(&mut m, Key::Right);
        assert!(m.prefs.stats.mode.on());
        key(&mut m, Key::Back);
        assert_eq!((m.view.top(), m.view.focus), (None, Focus::Strip(1)));
        assert!(m.strip_visible());
    }
    #[test]
    fn controller_opens_the_tvs_controller_settings() {
        let mut m = running();
        key(&mut m, Key::Right);
        key(&mut m, Key::Right);
        assert!(key(&mut m, Key::Ok).contains(&Command::OpenControllerSettings));
    }
    #[test]
    fn ending_asks_first_and_keep_playing_is_where_focus_starts() {
        let mut m = running();
        for _ in 0..4 {
            key(&mut m, Key::Right);
        }
        assert_eq!(m.view.focus, Focus::Strip(4));
        let out = key(&mut m, Key::Ok);
        assert_eq!((m.view.top(), m.view.focus), (Some(Layer::EndConfirm), Focus::SheetAction(0)));
        assert_eq!(routes(&out), [true], "a pad that opened it can answer it");
        assert!(m.session_active() && m.session().unwrap().phase() == &Phase::Running);
        key(&mut m, Key::Ok);
        assert_eq!((m.view.top(), m.view.focus), (None, Focus::Strip(4)), "OK on the safe default keeps playing");
        key(&mut m, Key::Ok);
        key(&mut m, Key::Right);
        let out = key(&mut m, Key::Ok);
        assert_eq!(m.session().unwrap().phase(), &Phase::Stopping);
        assert!(sent(&out).iter().any(|r| matches!(r, Request::Stop { .. })));
    }
    #[test]
    fn a_drawer_or_the_prompt_left_alone_for_half_a_minute_gives_the_game_back() {
        let mut m = running();
        key(&mut m, Key::Right);
        key(&mut m, Key::Ok);
        assert_eq!(m.view.top(), Some(Layer::Picture));
        tick(&mut m, 29_000);
        key(&mut m, Key::Down);
        tick(&mut m, 58_999);
        assert_eq!(m.view.top(), Some(Layer::Picture), "a key restarts the half minute");
        let out = tick(&mut m, 59_000);
        assert!(m.view.top().is_none() && routes(&out) == [false]);
        assert!(m.session_active(), "the prompt keeps playing, it does not end the session");
    }
    #[test]
    fn x_opens_the_steam_overlay_only_over_a_steam_copy() {
        let mut m = running();
        describe(&mut m, "gfn", vec![], &[Feature::EndGame, Feature::SteamOverlay]);
        m.step(Input::Overlay);
        assert!(key(&mut m, Key::Alt).is_empty() && m.strip_visible(), "not a Steam copy");
        assert!(Rules::hints(&m).0.is_empty());
        m.games[0].sources[0].entry.store = Some("steam".into());
        assert_eq!(Rules::hints(&m).0, [(Key::Alt, "Steam overlay")]);
        let out = key(&mut m, Key::Alt);
        let session = m.session().unwrap().id;
        assert_eq!(routes(&out), [false], "the game has the pad before the overlay opens");
        assert_eq!(out.last(), Some(&Command::SteamOverlay(Some((EngineId::new("gfn").unwrap(), session)))));
        assert!(!m.strip_visible());
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
    #[test]
    fn a_failed_launch_ends_on_the_game_page_with_the_engines_words_and_keeps_them_for_retry() {
        let mut m = launching("Game", false);
        report(&mut m, SessionState::Failed { message: "No seat free".into(), reason: ipc::FailReason::Busy });
        assert_eq!((m.view.page, m.view.notice.as_deref(), m.last_failure(GameId(1))), (Page::Game, Some("No seat free"), Some("No seat free")));
    }
}
