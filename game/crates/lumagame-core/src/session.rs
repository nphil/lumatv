//! One launch, from Play to its end.
//!
//! The machine holds no clock, engine, file or helper. The caller feeds it what happened ([`Input`]) with the time it happened, and
//! carries out what comes back ([`Effect`]); every decision about a session is here and runs in a test in microseconds.
//!
//! What it guarantees:
//! - A target is remembered, and playtime starts, only when the engine reports Running. Nothing before that leaves a trace.
//! - Low-latency mode is asked for when Running begins and given back when the session is over, after the stream has stopped: the
//!   picture is never restored under a live video plane.
//! - Every way out ends in exactly one [`Effect::Finished`]: the engine ending or failing, Back, the app going to the background,
//!   the engine's process dying, and an engine that stops answering.
//! - Leaving stops the stream, never the game, unless the owner asked to end it. A game left running where it can be picked up
//!   ends as Suspended.
//! - A game that is an app of its own takes the screen itself. LumaGame going to the background is then the game starting, not the
//!   owner leaving, and the picture is the TV's to manage: such a session ignores Background and never asks for low-latency mode.

use crate::engine::Presentation;
use crate::ids::{GameId, SessionId, SourceKey};
use crate::ipc::{EndReason, FailReason, SessionState};

/// An engine that reports nothing for this long while Checking is treated as stuck.
pub const CHECKING_TIMEOUT_MS: u64 = 30_000;
/// Starting covers a host waking and a cloud machine booting, so it is the long one. A queue has no limit: its engine owns that.
pub const STARTING_TIMEOUT_MS: u64 = 120_000;
/// How long an engine has to confirm a stop before its process is ended instead.
pub const STOPPING_TIMEOUT_MS: u64 = 10_000;
/// How often playtime is written down while playing, so a power cut loses at most this much.
pub const CHECKPOINT_MS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Checking,
    SignIn,
    Queue {
        position: Option<u32>,
        eta_s: Option<u32>,
    },
    Starting,
    Running,
    /// A stop was asked for and the engine has not confirmed it.
    Stopping,
    Done(Outcome),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ended(EndReason),
    Failed {
        reason: FailReason,
        message: String,
    },
    /// The owner backed out before the game was on screen.
    Cancelled,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// The engine reported this session's state.
    Engine(SessionState),
    /// Back, before the game is on screen.
    Cancel,
    /// The owner left a running game. `end_game` closes it on the far side too.
    Stop { end_game: bool },
    /// The app is no longer in front: Home was pressed, another app opened, the TV went to standby.
    Background,
    /// The engine's process exited or its pipe broke.
    EngineGone,
    /// Time passed. Sent about once a second.
    Tick,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Send `launch` to the engine.
    Launch,
    /// Send `stop` to the engine.
    Stop { end_game: bool },
    /// Take or give back the helper's low-latency lease. Never asked twice in the same direction.
    LowLatency(bool),
    /// The game is on screen: remember the target as the last that played it, and as pinned when `pin` is set.
    Started { pin: bool },
    /// Write the playtime so far to the checkpoint file.
    Checkpoint { seconds: u32 },
    /// The engine did not confirm a stop: end its process, which frees the video plane whatever state it is in.
    EndEngine,
    /// The session is over. `played` is `None` when it never ran.
    Finished { played: Option<u32>, outcome: Outcome },
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: SessionId,
    pub game: GameId,
    pub source: SourceKey,
    pin: bool,
    /// The game is an app of its own ([`Presentation::App`]).
    own_app: bool,
    phase: Phase,
    phase_since_ms: u64,
    /// When Running began, until the session stops running.
    running_since_ms: Option<u64>,
    /// Time spent Running, complete once the session has stopped running.
    ran_ms: Option<u64>,
    checkpoint_ms: u64,
    low_latency: bool,
    /// What Stopping ends as.
    stopping_as: Option<Outcome>,
}

impl Session {
    /// Begins a launch. `pin` is the chooser's "remember for this game".
    pub fn start(id: SessionId, game: GameId, source: SourceKey, presentation: Presentation, pin: bool, now_ms: u64) -> (Session, Vec<Effect>) {
        let session = Session {
            id,
            game,
            source,
            pin,
            own_app: presentation == Presentation::App,
            phase: Phase::Checking,
            phase_since_ms: now_ms,
            running_since_ms: None,
            ran_ms: None,
            checkpoint_ms: now_ms,
            low_latency: false,
            stopping_as: None,
        };
        (session, vec![Effect::Launch])
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn is_done(&self) -> bool {
        matches!(self.phase, Phase::Done(_))
    }

    /// Whole seconds spent Running so far; `None` before the game was on screen.
    pub fn played(&self, now_ms: u64) -> Option<u32> {
        let ms = match (self.ran_ms, self.running_since_ms) {
            (Some(ran), _) => ran,
            (None, Some(since)) => now_ms.saturating_sub(since),
            (None, None) => return None,
        };
        Some(u32::try_from(ms / 1000).unwrap_or(u32::MAX))
    }

    pub fn step(&mut self, now_ms: u64, input: Input) -> Vec<Effect> {
        let mut out = Vec::new();
        match (&self.phase, input) {
            (Phase::Done(_), _) => {}
            (_, Input::Background) if self.own_app => {}

            (Phase::Stopping, Input::Engine(SessionState::Ended { .. } | SessionState::Failed { .. }) | Input::EngineGone) => {
                let outcome = self.stopping_as.take().unwrap_or(Outcome::Ended(EndReason::Stopped));
                self.finish(now_ms, outcome, &mut out);
            }
            (Phase::Stopping, Input::Tick) if now_ms.saturating_sub(self.phase_since_ms) >= STOPPING_TIMEOUT_MS => {
                out.push(Effect::EndEngine);
                let outcome = self.stopping_as.take().unwrap_or(Outcome::Ended(EndReason::Stopped));
                self.finish(now_ms, outcome, &mut out);
            }
            // The stop is already on its way; a state that crossed it changes nothing.
            (Phase::Stopping, _) => {}

            (_, Input::Engine(SessionState::Ended { reason })) => self.finish(now_ms, Outcome::Ended(reason), &mut out),
            (_, Input::Engine(SessionState::Failed { reason, message })) => self.finish(now_ms, Outcome::Failed { reason, message }, &mut out),
            (_, Input::EngineGone) => {
                let outcome = Outcome::Failed { reason: FailReason::Engine, message: "The engine stopped unexpectedly.".into() };
                self.finish(now_ms, outcome, &mut out);
            }

            (Phase::Running, Input::Engine(_)) => {}
            (Phase::Running, Input::Stop { end_game }) => {
                let outcome = Outcome::Ended(if end_game { EndReason::Stopped } else { EndReason::Suspended });
                self.stop(now_ms, end_game, outcome, &mut out);
            }
            // Leaving the app stops the stream and leaves the game where it is.
            (Phase::Running, Input::Background | Input::Cancel) => self.stop(now_ms, false, Outcome::Ended(EndReason::Suspended), &mut out),
            (Phase::Running, Input::Tick) => {
                if now_ms.saturating_sub(self.checkpoint_ms) >= CHECKPOINT_MS {
                    self.checkpoint_ms = now_ms;
                    out.extend(self.played(now_ms).map(|seconds| Effect::Checkpoint { seconds }));
                }
            }

            (_, Input::Engine(SessionState::Running { .. })) => {
                self.enter(now_ms, Phase::Running);
                self.running_since_ms = Some(now_ms);
                self.checkpoint_ms = now_ms;
                if !self.own_app {
                    self.low_latency = true;
                    out.push(Effect::LowLatency(true));
                }
                out.push(Effect::Started { pin: self.pin });
            }
            (_, Input::Engine(SessionState::Checking)) => self.enter(now_ms, Phase::Checking),
            (_, Input::Engine(SessionState::SignIn)) => self.enter(now_ms, Phase::SignIn),
            (_, Input::Engine(SessionState::Queue { position, eta_s })) => self.enter(now_ms, Phase::Queue { position, eta_s }),
            (_, Input::Engine(SessionState::Starting)) => self.enter(now_ms, Phase::Starting),

            (_, Input::Cancel | Input::Stop { .. } | Input::Background) => self.stop(now_ms, false, Outcome::Cancelled, &mut out),
            (phase, Input::Tick) => {
                let limit = match phase {
                    Phase::Checking => Some(CHECKING_TIMEOUT_MS),
                    Phase::Starting => Some(STARTING_TIMEOUT_MS),
                    // The owner is reading a code or waiting in a line the engine reports on; neither is the engine being stuck.
                    _ => None,
                };
                if limit.is_some_and(|limit| now_ms.saturating_sub(self.phase_since_ms) >= limit) {
                    let outcome = Outcome::Failed { reason: FailReason::Timeout, message: "The engine did not answer in time.".into() };
                    self.stop(now_ms, false, outcome, &mut out);
                }
            }
        }
        out
    }

    /// Moves to a phase. Staying in one (a queue position changing) does not restart its clock.
    fn enter(&mut self, now_ms: u64, phase: Phase) {
        if std::mem::discriminant(&self.phase) != std::mem::discriminant(&phase) {
            self.phase_since_ms = now_ms;
        }
        self.phase = phase;
    }

    /// Playtime ends at the moment the stream is asked to stop, not when the engine gets round to confirming it.
    fn stop_running(&mut self, now_ms: u64) {
        if let Some(since) = self.running_since_ms.take() {
            self.ran_ms = Some(now_ms.saturating_sub(since));
        }
    }

    fn stop(&mut self, now_ms: u64, end_game: bool, outcome: Outcome, out: &mut Vec<Effect>) {
        self.stop_running(now_ms);
        self.stopping_as = Some(outcome);
        self.enter(now_ms, Phase::Stopping);
        out.push(Effect::Stop { end_game });
    }

    fn finish(&mut self, now_ms: u64, outcome: Outcome, out: &mut Vec<Effect>) {
        self.stop_running(now_ms);
        if std::mem::take(&mut self.low_latency) {
            out.push(Effect::LowLatency(false));
        }
        out.push(Effect::Finished { played: self.played(now_ms), outcome: outcome.clone() });
        self.enter(now_ms, Phase::Done(outcome));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Codec;
    use crate::ipc::StreamInfo;

    fn start_as(presentation: Presentation, pin: bool) -> Session {
        let (session, effects) = Session::start(SessionId(1), GameId(7), "gfn|acct|100".parse().unwrap(), presentation, pin, 0);
        assert_eq!(effects, [Effect::Launch]);
        session
    }

    fn start(pin: bool) -> Session {
        start_as(Presentation::Feeder, pin)
    }

    fn running() -> Input {
        Input::Engine(SessionState::Running {
            stream: Some(StreamInfo { codec: Codec::H265, width: 3840, height: 2160, fps: 120, hdr: true, audio_channels: 2, sdr_reason: None }),
        })
    }

    fn ended(reason: EndReason) -> Input {
        Input::Engine(SessionState::Ended { reason })
    }

    fn finished(played: Option<u32>, outcome: Outcome) -> Effect {
        Effect::Finished { played, outcome }
    }

    #[test]
    fn a_launch_goes_through_the_named_states_and_counts_only_the_time_it_ran() {
        let mut s = start(true);
        assert_eq!(s.step(1_000, Input::Engine(SessionState::Queue { position: Some(12), eta_s: None })), []);
        assert_eq!(s.step(9_000, Input::Engine(SessionState::Queue { position: Some(3), eta_s: Some(20) })), []);
        assert_eq!(*s.phase(), Phase::Queue { position: Some(3), eta_s: Some(20) });
        assert_eq!(s.step(30_000, Input::Engine(SessionState::Starting)), []);
        assert_eq!(s.played(40_000), None);

        assert_eq!(s.step(45_000, running()), [Effect::LowLatency(true), Effect::Started { pin: true }]);
        assert_eq!(s.step(46_000, running()), [], "said twice, started once");
        assert_eq!(s.step(104_000, Input::Tick), []);
        assert_eq!(s.step(105_000, Input::Tick), [Effect::Checkpoint { seconds: 60 }]);
        assert_eq!(s.step(106_000, Input::Tick), []);
        assert_eq!(s.step(165_500, Input::Tick), [Effect::Checkpoint { seconds: 120 }]);

        // The owner leaves. The game stays where it can be picked up, and the picture is restored only once the stream has stopped.
        assert_eq!(s.step(645_000, Input::Stop { end_game: false }), [Effect::Stop { end_game: false }]);
        assert_eq!(*s.phase(), Phase::Stopping);
        assert_eq!(s.step(647_000, running()), [], "a state that crossed the stop");
        assert_eq!(s.step(648_000, ended(EndReason::Stopped)), [Effect::LowLatency(false), finished(Some(600), Outcome::Ended(EndReason::Suspended))]);
        assert!(s.is_done());
        assert_eq!(s.step(650_000, running()), [], "nothing moves a finished session");
        assert_eq!(s.played(999_000), Some(600));
    }

    #[test]
    fn backing_out_before_the_game_is_on_screen_leaves_nothing_to_remember() {
        for (state, at) in [
            (SessionState::Checking, 500),
            (SessionState::SignIn, 500),
            (SessionState::Queue { position: Some(40), eta_s: None }, 500),
            (SessionState::Starting, 500),
        ] {
            let mut s = start(true);
            s.step(at, Input::Engine(state));
            assert_eq!(s.step(2_000, Input::Cancel), [Effect::Stop { end_game: false }]);
            assert_eq!(s.step(2_100, ended(EndReason::Stopped)), [finished(None, Outcome::Cancelled)]);
        }
        let mut s = start(false);
        assert_eq!(s.step(100, Input::Background), [Effect::Stop { end_game: false }], "Home while it is still starting");
        assert_eq!(s.step(200, Input::EngineGone), [finished(None, Outcome::Cancelled)]);
    }

    #[test]
    fn a_failure_or_a_far_side_end_finishes_at_once_whatever_the_phase() {
        let failed = SessionState::Failed { reason: FailReason::NotEntitled, message: "Not in this plan.".into() };
        let mut s = start(false);
        assert_eq!(
            s.step(3_000, Input::Engine(failed)),
            [finished(None, Outcome::Failed { reason: FailReason::NotEntitled, message: "Not in this plan.".into() })]
        );

        let mut s = start(false);
        s.step(1_000, running());
        assert_eq!(s.step(61_000, ended(EndReason::Remote)), [Effect::LowLatency(false), finished(Some(60), Outcome::Ended(EndReason::Remote))]);

        let mut s = start(false);
        s.step(1_000, running());
        let gone = s.step(31_900, Input::EngineGone);
        assert!(matches!(
            gone.as_slice(),
            [Effect::LowLatency(false), Effect::Finished { played: Some(30), outcome: Outcome::Failed { reason: FailReason::Engine, .. } }]
        ));
    }

    #[test]
    fn leaving_the_app_stops_the_stream_and_keeps_the_game() {
        let mut s = start(false);
        s.step(1_000, running());
        assert_eq!(s.step(11_000, Input::Background), [Effect::Stop { end_game: false }]);
        assert_eq!(s.step(11_400, ended(EndReason::Stopped)), [Effect::LowLatency(false), finished(Some(10), Outcome::Ended(EndReason::Suspended))]);

        let mut s = start(false);
        s.step(1_000, running());
        assert_eq!(s.step(11_000, Input::Stop { end_game: true }), [Effect::Stop { end_game: true }]);
        assert_eq!(s.step(11_400, ended(EndReason::Stopped)), [Effect::LowLatency(false), finished(Some(10), Outcome::Ended(EndReason::Stopped))]);
    }

    #[test]
    fn an_engine_that_stops_answering_is_timed_out_and_then_ended() {
        let timeout = || Outcome::Failed { reason: FailReason::Timeout, message: "The engine did not answer in time.".into() };

        let mut s = start(false);
        assert_eq!(s.step(CHECKING_TIMEOUT_MS - 1, Input::Tick), []);
        assert_eq!(s.step(CHECKING_TIMEOUT_MS, Input::Tick), [Effect::Stop { end_game: false }]);
        assert_eq!(s.step(CHECKING_TIMEOUT_MS + STOPPING_TIMEOUT_MS - 1, Input::Tick), []);
        assert_eq!(s.step(CHECKING_TIMEOUT_MS + STOPPING_TIMEOUT_MS, Input::Tick), [Effect::EndEngine, finished(None, timeout())]);

        let mut s = start(false);
        s.step(1_000, Input::Engine(SessionState::Starting));
        assert_eq!(s.step(1_000 + STARTING_TIMEOUT_MS - 1, Input::Tick), []);
        assert_eq!(s.step(1_000 + STARTING_TIMEOUT_MS, Input::Tick), [Effect::Stop { end_game: false }]);
        assert_eq!(s.step(1_500 + STARTING_TIMEOUT_MS, ended(EndReason::Stopped)), [finished(None, timeout())]);

        // A queue and a sign-in wait as long as they take.
        let mut s = start(false);
        s.step(1_000, Input::Engine(SessionState::Queue { position: Some(900), eta_s: None }));
        assert_eq!(s.step(3_600_000, Input::Tick), []);
        s.step(3_600_000, Input::Engine(SessionState::SignIn));
        assert_eq!(s.step(7_200_000, Input::Tick), []);

        // A running game whose engine ignores the stop: the time counts up to the stop, and the process is ended.
        let mut s = start(false);
        s.step(1_000, running());
        s.step(21_000, Input::Stop { end_game: false });
        assert_eq!(
            s.step(21_000 + STOPPING_TIMEOUT_MS, Input::Tick),
            [Effect::EndEngine, Effect::LowLatency(false), finished(Some(20), Outcome::Ended(EndReason::Suspended))]
        );
    }

    #[test]
    fn a_game_that_is_its_own_app_takes_the_screen_without_ending_the_session() {
        let mut s = start_as(Presentation::App, false);
        assert_eq!(s.step(300, Input::Background), [], "the game's app came to the front");
        assert_eq!(s.step(400, Input::Engine(SessionState::Running { stream: None })), [Effect::Started { pin: false }]);
        assert_eq!(s.step(60_400, Input::Background), []);
        assert_eq!(s.step(60_400, Input::Tick), [Effect::Checkpoint { seconds: 60 }]);
        assert_eq!(s.step(90_400, ended(EndReason::Remote)), [finished(Some(90), Outcome::Ended(EndReason::Remote))]);
    }
}
