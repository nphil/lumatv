//! The route: readiness, arming, monitoring and restoration of LG's audio path for one passthrough session at a time.
//!
//! The route is applied per session and fully put back when the session ends, whatever ends it: the player releasing it, the
//! player dying, LG changing the audio path, the plugin switched off, the TV going to standby, or the helper stopping. Between
//! sessions the TV is stock. A session is three changes, in this order, each journalled before it is made:
//!
//! 1. LG's PCM bypass converter is told to carry 8 channels of 16-bit 192 kHz PCM (`CONVERTER_COMMAND`): the encoded stream
//!    rides inside that PCM, and LG's renderer passes the words through untouched.
//! 2. The eARC output words are un-mixed (`Mix_option` 1 to 0) so LG stops mixing its system sounds into that stream.
//! 3. Once the kernel reports `Pass-Through` and the stream has been flowing for [`CONNECT_DWELL_MS`], the ADEC the player's
//!    renderer was granted is connected to the main output and, when the kernel holds it muted, unmuted with a mute true/false
//!    pair (audiooutputd skips a lone unmute). That last step is what makes the TV's eARC transmitter send the stream to the
//!    receiver.
//!
//! The ADEC comes from the player, never from an assumption, and a session starts only while audiooutputd and the kernel show
//! nothing connected and the decoder stopped, so no HDMI input's decoder is touched. The words are only written back while they
//! still hold what this route wrote. The ADEC is only muted and disconnected while it is not decoding: its `Connect=` reads the
//! same (`HDMI_PORT2` on ADEC1) before, during and after a session, so it cannot tell LG's connection from ours, but `Start=` stayed 0
//! through every measured session. An HDMI input that starts decoding on the same ADEC is therefore left as LG has it.
//!
//! Ending reverses 3 and 2: mute, put the mute back as the session found it once the player has closed its renderer, disconnect,
//! put the words back. Then the TV is compared with what the session found (audiooutputd's connection list, the kernel's
//! connected inputs, the ADEC's own kernel line) and LG's input services are checked (no audio task wedged in the sound card's
//! control lock, the HDMI input service answers a ping); a fault holds the next session off. A helper that is stopping has seconds
//! (`hurry`): it does not wait for the renderer or compare afterwards, and what it cannot finish stays journalled for the next start.
//! A running session is watched from /proc and the kernel only. A step of the watch that waits on LG (a luna call, a health check, a
//! read-back) reads the kernel again and publishes the reading about every [`crate::host::PUMP_MS`] while it waits, so the reading a
//! player polls is never old by the length of a luna call.
//!
//! Ported from `lgroute.c` and `lgroute-control.c` of dts_restore_plus (LGPL-2.1-or-later): the converter command, the words and
//! their ranges, and the rule that gives up once LG has mixed or re-encoded the stream for about a second.

use crate::firmware::{self, Firmware};
use crate::graph::{self, Chain};
use crate::health::{self, Health, returned_true};
use crate::host::{Host, WriteError};
use crate::journal::Journal;
use crate::sndout::{self, Adec, Sndout, Words};
use std::sync::Arc;

pub const AREN: &str = "/dev/lg/aren0";
pub const GRAPH: &str = "/proc/aaudio/dot_module_connection";
pub const INSTANCES: &str = "/proc/aaudio/instances";
pub const BOOT_ID: &str = "/proc/sys/kernel/random/boot_id";
const AUDIO: &str = "luna://com.webos.service.audiooutput/audio/";
/// Why LG's sound status cannot be read while an earlier read of the driver's files is still waiting.
const DRIVER_BUSY: &str = "LG's audio driver is busy: a read of its files has not returned yet";
/// 8 channels of 16-bit PCM at 192 kHz, which carry an encoded stream in IEC 61937 bursts (lgroute.c).
const CONVERTER_COMMAND: &str = "0x50020020 192000 8 16 0 0 192000 8 16\n";

/// How long the stream must have been flowing before the ADEC is connected. The kernel cannot tell a stream from silence, and the
/// player starts the stream the moment the route reports bypass; the proven window connected 3 s after that.
const CONNECT_DWELL_MS: u64 = 3000;
/// Measured on the C4: mute true 2.8 s after the connect, mute false 0.7 s after that. Shorter gaps are untested.
const MUTE_AFTER_CONNECT_MS: u64 = 2800;
const UNMUTE_AFTER_MUTE_MS: u64 = 700;
/// LG's policy rewrites can mix for a moment; the route gives up when it has not cleared after this long (lgroute.c).
const LOSS_GRACE_MS: u64 = 1000;
/// A flush or track switch makes LG rebuild the renderer; the player re-arms once the new one is there.
const REARM_GRACE_MS: u64 = 5000;
/// Between disconnecting the ADEC and putting the words back.
const SETTLE_MS: u64 = 300;
const HEALTH_RETRY_MS: u64 = 1500;
const ARMED_TICK_MS: u64 = 100;
const ARMED_HEALTH_MS: u64 = 2000;
const IDLE_TICK_MS: u64 = 2000;
const IDLE_HEALTH_MS: u64 = 10_000;
/// The highest decoder input number LG's resource manager hands out on the C4 (`adec.p0` to `adec.p7`).
pub const MAX_ADEC: u8 = 7;

#[derive(Clone, Copy)]
struct Wait {
    polls: u32,
    every_ms: u64,
}
/// LG's renderer takes up to a couple of seconds to join the graph.
const GRAPH_WAIT: Wait = Wait {
    polls: 40,
    every_ms: 50,
};
const CONVERTER_WAIT: Wait = Wait {
    polls: 20,
    every_ms: 100,
};
/// A kernel read-back after a write.
const VERIFY_WAIT: Wait = Wait {
    polls: 10,
    every_ms: 100,
};
/// How long a restore waits for the player to close its renderer before it leaves the input muted instead of unmuting it.
const RENDERER_CLOSE_WAIT: Wait = Wait {
    polls: 20,
    every_ms: 100,
};

/// What the caller knows that the route cannot see.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    pub enabled: bool,
    pub screen_on: bool,
    /// The helper is shutting down.
    pub stopping: bool,
    /// LG's saved sound output (`external_arc` for the receiver on eARC), `None` until it has been read.
    pub sound_output: Option<String>,
    /// LG's digital sound output setting; the route runs best on `passThrough`. Reported, never required.
    pub digital: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    /// The firmware is not the proven build.
    Refused,
    NotReady,
    Ready,
    Armed,
    /// A session ended because the route broke. Stays until the next arm or release; `ready` can be true meanwhile.
    Lost,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Refused => "refused",
            Self::NotReady => "not-ready",
            Self::Ready => "ready",
            Self::Armed => "armed",
            Self::Lost => "lost",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// The converter is configured and the words un-mixed; the kernel has not reported Pass-Through yet.
    Armed,
    /// The kernel reports Pass-Through: the player may send the stream.
    Bypassing,
    /// The ADEC is connected to the main output.
    Connected,
    /// The mute true/false pair is under way.
    Unmuting,
    /// The ADEC is unmuted: the receiver is being fed.
    Audible,
    /// The session is being put back.
    Ending,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Self::Armed => "armed",
            Self::Bypassing => "bypassing",
            Self::Connected => "connected",
            Self::Unmuting => "unmuting",
            Self::Audible => "audible",
            Self::Ending => "ending",
        }
    }

    fn connected(self) -> bool {
        matches!(self, Self::Connected | Self::Unmuting | Self::Audible)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The player ended the session.
    Released,
    /// The player's process is gone.
    OwnerExited,
    /// LG changed the audio path under the session.
    RouteLost,
    Disabled,
    Standby,
    HelperExit,
    /// The helper's own thread failed.
    Fault,
    /// A journal an earlier helper left was replayed at start.
    Recovered,
}

impl Why {
    pub fn name(self) -> &'static str {
        match self {
            Self::Released => "released",
            Self::OwnerExited => "owner-exited",
            Self::RouteLost => "route-lost",
            Self::Disabled => "disabled",
            Self::Standby => "standby",
            Self::HelperExit => "helper-exit",
            Self::Fault => "fault",
            Self::Recovered => "recovered",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct End {
    pub why: Why,
    pub detail: String,
}

/// An arm the route did not accept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub reason: String,
    /// The same request may succeed in a moment (LG's graph still building, audio services busy).
    pub retry: bool,
}

impl Refusal {
    fn now(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            retry: false,
        }
    }

    fn later(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            retry: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Observed {
    at: u64,
    bypassing: bool,
    mixed: bool,
    converter_present: bool,
    /// Why not bypassing; empty when bypassing.
    why: String,
}

struct Session {
    pid: u32,
    start: u64,
    tail: String,
    converter: String,
    journal: Journal,
    phase: Phase,
    since: u64,
    observed: Observed,
    /// When the route first stopped looking like bypass, for the grace period.
    bad_since: Option<u64>,
}

/// A session's state for the topic and the route endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionView {
    pub pid: u32,
    pub tail: String,
    pub converter: String,
    /// The decoder input the session connects: the one LG granted the player's renderer.
    pub adec: u8,
    pub phase: Phase,
    pub bypassing: bool,
    pub mixed: bool,
    pub converter_present: bool,
    pub connected: bool,
    pub audible: bool,
    /// Why not bypassing; empty when bypassing.
    pub why: String,
    /// When this was last read, on the clock of [`Host::now_ms`].
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub state: State,
    /// Whether an arm would be accepted now.
    pub ready: bool,
    pub reason: String,
    pub firmware: Option<Firmware>,
    pub earc: Option<bool>,
    pub words: Option<String>,
    /// `[ARC] OutputCodec`.
    pub codec: Option<String>,
    pub output: Option<String>,
    pub digital: Option<String>,
    pub session: Option<SessionView>,
    pub end: Option<End>,
    /// A restore that could not finish and why.
    pub pending: Option<String>,
    /// What the last session left different from what it found, and a silent HDMI input service; until the next arm.
    pub after: Option<String>,
    pub at: u64,
}

/// Puts one session's changes back. Returns what is still outstanding.
enum Restore {
    Done,
    Pending(String),
}

/// Why [`Route::apply_words`] did not end with the words read back as asked.
struct Unapplied {
    why: String,
    /// The write was made, so LG's words may have changed all the same.
    written: bool,
}

pub struct Route {
    host: Arc<dyn Host>,
    journal_path: String,
    env: Env,
    session: Option<Session>,
    /// The reason of the last loss, until the next arm or release.
    lost: Option<String>,
    end: Option<End>,
    /// A restore that is waiting for the audio stack, and why.
    pending: Option<(Journal, String)>,
    /// What the last restore found different from what the session found, until the next arm.
    after: Option<String>,
    /// What is wrong with LG's input services after the last restore (a wedged audio task, a silent HDMI input service): no session
    /// starts until they are healthy again.
    input_fault: Option<String>,
    /// An `amixer` write that did not answer: nothing touches the route again until the helper restarts.
    hung: Option<String>,
    firmware: Option<Firmware>,
    sndout: Option<Sndout>,
    ready: Result<(), String>,
    health: Health,
    health_at: Option<u64>,
    /// A monitoring step is running: its waits publish a fresh reading. An arm, a re-arm and a restore do not, as the player waits
    /// for those calls to return.
    monitoring: bool,
    observer: Option<Observer>,
}

/// Called with every state change.
type Observer = Box<dyn Fn(&Snapshot) + Send + Sync>;

impl Route {
    pub fn new(host: Arc<dyn Host>, journal_path: impl Into<String>) -> Self {
        Self {
            host,
            journal_path: journal_path.into(),
            env: Env::default(),
            session: None,
            lost: None,
            end: None,
            pending: None,
            after: None,
            input_fault: None,
            hung: None,
            firmware: None,
            sndout: None,
            ready: Err("starting".into()),
            health: Health::Ok,
            health_at: None,
            monitoring: false,
            observer: None,
        }
    }

    /// Called with a fresh snapshot after every step, from inside the step.
    pub fn observe_with(&mut self, observer: impl Fn(&Snapshot) + Send + Sync + 'static) {
        self.observer = Some(Box::new(observer));
    }

    fn notify(&self) {
        if let Some(observer) = &self.observer {
            observer(&self.snapshot());
        }
    }

    fn boot_id(&self) -> String {
        self.host
            .read(BOOT_ID)
            .map(|id| id.trim().to_owned())
            .unwrap_or_default()
    }

    fn read_sndout(&self) -> Option<Sndout> {
        self.host
            .read(sndout::SNDOUT)
            .ok()
            .map(|text| sndout::parse(&text))
    }

    fn read_adec(&self, index: u8) -> Option<Adec> {
        self.host
            .read(&sndout::adec_path(index))
            .ok()
            .map(|text| sndout::parse_adec(&text))
    }

    /// Reads the kernel again and publishes the reading during a monitoring step. A step that waits on LG (a luna call, a health
    /// check, a wait for the kernel) takes longer than the 300 ms a player tolerates, and the reading it polls would be older than
    /// that when the step ends.
    fn publish_reading(&mut self) {
        if self.monitoring {
            self.observe();
            self.notify();
        }
    }

    /// Sleeps `ms`; during a monitoring step a fresh reading is published about every [`crate::host::PUMP_MS`] of it.
    fn nap(&mut self, ms: u64) {
        if !self.monitoring {
            self.host.sleep_ms(ms);
            return;
        }
        let host = self.host.clone();
        host.sleep_pumped(ms, &mut || self.publish_reading());
    }

    /// Looks every `wait.every_ms` until `done` says yes, up to `wait.polls` times.
    fn poll(&mut self, wait: Wait, mut done: impl FnMut(&Self) -> bool) -> bool {
        for _ in 0..wait.polls {
            if done(self) {
                return true;
            }
            self.nap(wait.every_ms);
        }
        done(self)
    }

    fn save_journal(&self, journal: &Journal) -> Result<(), String> {
        self.host
            .save(&self.journal_path, &journal.render())
            .map_err(|e| format!("cannot write the recovery journal: {e}"))
    }

    fn luna(&mut self, verb: &str, payload: &str) -> bool {
        let host = self.host.clone();
        host.luna_pumped(&format!("{AUDIO}{verb}"), payload, &mut || {
            self.publish_reading()
        })
        .is_some_and(|reply| returned_true(&reply))
    }

    fn luna_mute(&mut self, adec: u8, mute: bool) -> bool {
        self.luna(
            "mute",
            &format!(r#"{{"source":"ADEC{adec}","sink":"MAIN","mute":{mute}}}"#),
        )
    }

    /// `connect` or `disconnect` of the ADEC to the main output.
    fn luna_link(&mut self, verb: &str, adec: u8) -> bool {
        self.luna(verb, &format!(r#"{{"source":"ADEC{adec}","sink":"MAIN"}}"#))
    }

    // ---- readiness ------------------------------------------------------------------------------------------------------

    /// Why an arm would not be accepted, from what is known now. Does not look at health or at the player.
    fn conditions(&self, snd: Option<&Sndout>, rearm: bool) -> Result<(), String> {
        if let Some(Err(why)) = self.firmware.as_ref().map(|fw| &fw.verdict) {
            return Err(why.clone());
        }
        if !self.env.screen_on {
            return Err("the TV is in standby".into());
        }
        match self.env.sound_output.as_deref() {
            Some("external_arc") => {}
            Some(other) => {
                return Err(format!(
                    "sound output is {other}; the route needs the receiver on eARC (external_arc)"
                ));
            }
            None => return Err("LG's sound output setting has not been read yet".into()),
        }
        let snd = snd.ok_or_else(|| {
            if self.host.driver_busy() {
                DRIVER_BUSY
            } else {
                "LG's sound status cannot be read"
            }
        })?;
        match snd.earc_on {
            Some(true) => {}
            Some(false) => return Err("eARC is off".into()),
            None => return Err("eARC state cannot be read".into()),
        }
        match snd.earc {
            Some(words) if words.valid() => {}
            Some(words) => {
                return Err(format!(
                    "LG's eARC output words ({words}) are not a known state"
                ));
            }
            None => return Err("LG's eARC output words cannot be read".into()),
        }
        if !rearm && !snd.connected_adecs().is_empty() {
            return Err(format!(
                "LG is routing audio through ADEC{} (an HDMI input or an app is using the audio output)",
                snd.connected_adecs().join(", ADEC")
            ));
        }
        Ok(())
    }

    /// A fresh check before a step that changes the route. One retry after a moment when only something transient is busy.
    fn health_gate(&mut self) -> Result<(), Refusal> {
        if let Some(why) = &self.hung {
            return Err(Refusal::now(why.clone()));
        }
        let mut now = self.check_health();
        if matches!(now, Health::Busy(_)) {
            self.nap(HEALTH_RETRY_MS);
            now = self.check_health();
        }
        self.health = now.clone();
        self.health_at = Some(self.host.now_ms());
        match now {
            Health::Ok => Ok(()),
            Health::Busy(why) => Err(Refusal::later(format!("audio services are busy: {why}"))),
            Health::Wedged(why) => Err(Refusal::now(format!(
                "audio control is stuck: {why}; the TV needs a restart"
            ))),
        }
    }

    /// [`health::check`], publishing a fresh reading meanwhile during a monitoring step.
    fn check_health(&mut self) -> Health {
        let host = self.host.clone();
        health::check_pumped(&*host, &mut || self.publish_reading())
    }

    fn refresh_idle(&mut self) {
        let now = self.host.now_ms();
        // An identity that could not be read says nothing about the build and is read again; a verdict is kept.
        if self.firmware.as_ref().is_none_or(|fw| fw.unread) {
            self.firmware = Some(firmware::check(&*self.host));
        }
        self.sndout = self.read_sndout();
        if self
            .health_at
            .is_none_or(|at| now.saturating_sub(at) >= IDLE_HEALTH_MS)
            && self.hung.is_none()
        {
            self.health = health::check(&*self.host);
            self.health_at = Some(now);
            if self.input_fault.is_some() {
                self.probe_input_services();
            }
        }
        self.ready = if let Some((_, note)) = &self.pending {
            Err(note.clone())
        } else if let Some(why) = &self.hung {
            Err(why.clone())
        } else if let Some(why) = &self.input_fault {
            Err(why.clone())
        } else if let Health::Wedged(why) | Health::Busy(why) = &self.health {
            Err(format!("audio services are not healthy: {why}"))
        } else {
            self.conditions(self.sndout.as_ref(), false)
        };
    }

    // ---- arming ---------------------------------------------------------------------------------------------------------

    /// The player's start time, once it is shown to hold exactly one `/dev/lg/aren0`.
    fn owner(&self, pid: u32) -> Result<u64, String> {
        if pid <= 1 {
            return Err(format!("{pid} is not a player process"));
        }
        let start = self
            .host
            .start_time(pid)
            .ok_or_else(|| format!("process {pid} is not running"))?;
        let held = self
            .host
            .fd_targets(pid)
            .map_err(|e| format!("cannot read process {pid}'s descriptors: {e}"))?
            .iter()
            .filter(|target| target.as_str() == AREN)
            .count();
        if held != 1 {
            return Err(format!(
                "process {pid} holds {held} {AREN} descriptors, not exactly one"
            ));
        }
        Ok(start)
    }

    /// The converter feeding `tail`'s lipsync stage, once LG's graph shows exactly one.
    fn converter_for(&mut self, tail: &str) -> Result<String, Refusal> {
        if !self.poll(GRAPH_WAIT, |route| {
            route
                .host
                .read(GRAPH)
                .is_ok_and(|g| graph::has_lipsync(&g, tail))
        }) {
            return Err(Refusal::later(format!(
                "LG's graph has no lipsync_{tail} stage yet"
            )));
        }
        let mut found = Chain::Missing;
        self.poll(CONVERTER_WAIT, |route| {
            found =
                graph::bypass_converter(&route.host.read(GRAPH).unwrap_or_default(), Some(tail));
            matches!(&found, Chain::One { converter, .. } if route.host.is_dir(&format!("{INSTANCES}/{converter}")))
        });
        match found {
            Chain::One { converter, .. }
                if self.host.is_dir(&format!("{INSTANCES}/{converter}")) =>
            {
                Ok(converter)
            }
            Chain::One { converter, .. } => Err(Refusal::later(format!(
                "converter {converter} has no instance directory"
            ))),
            Chain::Missing => Err(Refusal::later(format!(
                "no PCM bypass converter feeds lipsync_{tail}"
            ))),
            Chain::Several(n) => Err(Refusal::later(format!(
                "{n} converters feed lipsync_{tail}; refusing to guess"
            ))),
        }
    }

    /// Puts the eARC words to `words` and waits for the kernel to report them.
    fn apply_words(&mut self, words: Words) -> Result<(), Unapplied> {
        match self.host.set_earc_words(words) {
            Ok(()) => {}
            Err(WriteError::Hung) => {
                let why = "a sound card write did not answer; restart the helper once the TV's audio is healthy".to_owned();
                self.hung = Some(why.clone());
                return Err(Unapplied { why, written: true });
            }
            Err(WriteError::Failed(why)) => {
                return Err(Unapplied {
                    why,
                    written: false,
                });
            }
        }
        if self.poll(VERIFY_WAIT, |route| {
            route.read_sndout().and_then(|s| s.earc) == Some(words)
        }) {
            Ok(())
        } else {
            Err(Unapplied {
                why: format!("LG did not take the eARC words {words}"),
                written: true,
            })
        }
    }

    /// Starts a session for `pid`, whose renderer's lipsync stage is `tail` and whose sink LG granted the decoder input `adec`, or
    /// re-arms the one it already has (a flush, seek or track switch makes LG build a new renderer). Returns the converter that
    /// was configured.
    pub fn arm(&mut self, pid: u32, tail: &str, adec: u8, env: &Env) -> Result<String, Refusal> {
        self.env = env.clone();
        let result = self.arm_inner(pid, tail, adec);
        self.notify();
        result
    }

    fn arm_inner(&mut self, pid: u32, tail: &str, adec: u8) -> Result<String, Refusal> {
        if !self.env.enabled {
            return Err(Refusal::now("the plugin is switched off"));
        }
        if tail.len() != 8 || !tail.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(Refusal::now("tail must be 8 lowercase hex digits"));
        }
        if adec > MAX_ADEC {
            return Err(Refusal::now(format!(
                "ADEC{adec} is not a decoder input of this TV (ADEC0 to ADEC{MAX_ADEC})"
            )));
        }
        let rearm = match &self.session {
            Some(s) if s.pid != pid => {
                return Err(Refusal::now(format!("process {} holds the route", s.pid)));
            }
            Some(s) if s.journal.adec != adec => {
                return Err(Refusal::now(format!(
                    "the session holds ADEC{}, not ADEC{adec}",
                    s.journal.adec
                )));
            }
            Some(_) => true,
            None => false,
        };
        if !rearm {
            self.retry_pending(false);
            if let Some((_, note)) = &self.pending {
                return Err(Refusal::now(format!(
                    "an earlier session's settings are not back yet: {note}"
                )));
            }
            // A journal nobody replayed holds LG's original words; a new one saved over it would lose them.
            if self.host.read(&self.journal_path).is_ok() {
                return Err(Refusal::now(
                    "an earlier session's journal has not been replayed",
                ));
            }
            if self.input_fault.is_some() {
                self.probe_input_services();
                if let Some(why) = &self.input_fault {
                    return Err(Refusal::now(why.clone()));
                }
            }
        }
        let start = self.owner(pid).map_err(Refusal::now)?;
        let firmware = firmware::check(&*self.host);
        let (verdict, unread) = (firmware.verdict.clone(), firmware.unread);
        self.firmware = Some(firmware);
        if let Err(why) = verdict {
            // A file that could not be read says nothing about the build: the same request may succeed in a moment.
            return Err(if unread {
                Refusal::later(why)
            } else {
                Refusal::now(why)
            });
        }
        self.health_gate()?;
        let snd = self.read_sndout();
        self.conditions(snd.as_ref(), rearm)
            .map_err(Refusal::later)?;
        let words = snd
            .as_ref()
            .and_then(|s| s.earc)
            .ok_or_else(|| Refusal::later("eARC words cannot be read"))?;
        let converter = self.converter_for(tail)?;

        let journal = match &self.session {
            Some(s) => s.journal.clone(),
            None => Journal {
                boot: self.boot_id(),
                pid,
                before: words,
                applied: words.unmixed(),
                adec,
                kernel: self.idle_adec(adec)?,
                connected: false,
            },
        };
        if !rearm {
            self.save_journal(&journal).map_err(Refusal::now)?;
        }
        let drop_journal = |route: &Self| {
            if !rearm {
                let _ = route.host.remove(&route.journal_path);
            }
        };
        if let Err(e) = self
            .host
            .write(&format!("{INSTANCES}/{converter}/cmd"), CONVERTER_COMMAND)
        {
            drop_journal(self);
            return Err(Refusal::later(format!(
                "cannot configure converter {converter}: {e}"
            )));
        }
        if words.mix != 0 {
            if let Err(unapplied) = self.apply_words(journal.applied) {
                if unapplied.written && !rearm {
                    // The write was made, so LG's words may have changed although they were not read back (the driver did not
                    // answer, or LG did not take them): the journal stays, and the restore puts them back once they read again.
                    self.pending = Some((journal, unapplied.why.clone()));
                } else {
                    drop_journal(self);
                }
                return Err(Refusal::now(unapplied.why));
            }
        }

        let now = self.host.now_ms();
        let (phase, since) = match self.session.take() {
            Some(old) => (old.phase, old.since),
            None => (Phase::Armed, now),
        };
        self.session = Some(Session {
            pid,
            start,
            tail: tail.to_owned(),
            converter: converter.clone(),
            journal,
            phase,
            since,
            observed: Observed::default(),
            bad_since: None,
        });
        self.lost = None;
        self.end = None;
        self.after = None;
        self.sndout = self.read_sndout();
        self.observe();
        Ok(converter)
    }

    /// The granted ADEC as LG left it, once nothing else is using audio: nothing connected in audiooutputd or the kernel and the
    /// decoder stopped. Its kernel line is what a restore puts back and is compared with.
    fn idle_adec(&self, adec: u8) -> Result<String, Refusal> {
        let kernel = self
            .read_adec(adec)
            .filter(|a| a.connect.is_some() && a.mute.is_some())
            .ok_or_else(|| Refusal::now(format!("ADEC{adec} cannot be read on this TV")))?;
        if kernel.start == Some(true) {
            return Err(Refusal::later(format!("ADEC{adec} is decoding for LG")));
        }
        match health::connections(&*self.host) {
            None => Err(Refusal::later(
                "the audio service's connections cannot be read",
            )),
            Some(list) if !list.is_empty() => Err(Refusal::later(format!(
                "LG is routing audio ({}); an HDMI input or an app is playing",
                list.join(", ")
            ))),
            Some(_) => Ok(kernel.render()),
        }
    }

    // ---- observing and monitoring ---------------------------------------------------------------------------------------

    /// Reads the kernel and records what the session looks like now.
    fn observe(&mut self) {
        let Some(converter) = self.session.as_ref().map(|s| s.converter.clone()) else {
            return;
        };
        let snd = self.read_sndout();
        let present = self.host.is_dir(&format!("{INSTANCES}/{converter}"));
        let why = match &snd {
            None if self.host.driver_busy() => DRIVER_BUSY,
            None => "LG's sound status cannot be read",
            Some(_) if !present => "LG rebuilt its audio path: the converter is gone",
            Some(s) if s.earc_on != Some(true) => "eARC is off",
            Some(s) if s.mixed() => "LG turned eARC mixing back on",
            Some(s) if !s.bypassing() => "the kernel is not reporting Pass-Through",
            Some(_) => "",
        };
        let observed = Observed {
            at: self.host.now_ms(),
            bypassing: why.is_empty(),
            mixed: snd.as_ref().is_some_and(Sndout::mixed),
            converter_present: present,
            why: why.to_owned(),
        };
        self.sndout = snd;
        if let Some(s) = &mut self.session {
            s.observed = observed;
        }
    }

    /// One monitoring step of a live session. Returns why it must end, if it must.
    fn monitor(&mut self) -> Option<(Why, String)> {
        let now = self.host.now_ms();
        if self
            .health_at
            .is_none_or(|at| now.saturating_sub(at) >= ARMED_HEALTH_MS)
        {
            // Tasks only, from /proc: a luna call here would hold the route for as long as LG takes to answer. The scan cannot be
            // cut into slices, so the reading is refreshed on either side of it.
            let host = self.host.clone();
            self.health = health::tasks_pumped(&*host, &mut || self.publish_reading());
            self.health_at = Some(now);
            if let Health::Wedged(why) = &self.health {
                return Some((Why::RouteLost, format!("audio control is stuck: {why}")));
            }
        }
        self.observe();
        let (observed, phase, adec, base) = {
            let s = self.session.as_mut()?;
            (
                s.observed.clone(),
                s.phase,
                s.journal.adec,
                sndout::parse_adec(&s.journal.kernel),
            )
        };
        let snd_ok = self.sndout.is_some();
        if !snd_ok || !observed.bypassing {
            let since = *self.session.as_mut()?.bad_since.get_or_insert(now);
            let grace = if observed.converter_present {
                LOSS_GRACE_MS
            } else {
                REARM_GRACE_MS
            };
            if now.saturating_sub(since) >= grace {
                return Some((Why::RouteLost, observed.why));
            }
        } else if let Some(s) = &mut self.session {
            s.bad_since = None;
        }
        if phase.connected() {
            let connected = self.sndout.as_ref().is_some_and(|s| s.adec_connected(adec));
            let other = self.sndout.as_ref().and_then(|s| {
                s.connected_adecs()
                    .into_iter()
                    .find(|a| *a != adec.to_string())
                    .map(str::to_owned)
            });
            let kernel = self.read_adec(adec);
            if !connected {
                return Some((Why::RouteLost, "LG disconnected the audio input".into()));
            }
            if let Some(other) = other {
                return Some((
                    Why::RouteLost,
                    format!("LG connected ADEC{other} while the session held ADEC{adec}"),
                ));
            }
            if let Some(a) = kernel.as_ref().filter(|a| lg_uses(a, &base)) {
                return Some((
                    Why::RouteLost,
                    format!("LG is using ADEC{adec} itself: it reads {}", a.render()),
                ));
            }
            if phase == Phase::Audible && kernel.as_ref().and_then(|a| a.mute) == Some(true) {
                return Some((Why::RouteLost, "LG muted the audio input".into()));
            }
        }
        let was = phase;
        let ended = self.advance(now);
        if ended.is_none() && self.session.as_ref().is_some_and(|s| s.phase != was) {
            // A step that talks to LG takes a while, and the player stops sending once the reading it polls is older than 300 ms:
            // read again after the step, so the stamp is not from before it.
            self.observe();
        }
        ended
    }

    /// Moves the connect sequence on by at most one step.
    fn advance(&mut self, now: u64) -> Option<(Why, String)> {
        let (phase, since, bypassing, adec) = {
            let s = self.session.as_ref()?;
            (s.phase, s.since, s.observed.bypassing, s.journal.adec)
        };
        let set = |route: &mut Self, phase: Phase| {
            if let Some(s) = &mut route.session {
                s.phase = phase;
                s.since = now;
            }
        };
        match phase {
            Phase::Armed if bypassing => set(self, Phase::Bypassing),
            Phase::Bypassing if !bypassing => set(self, Phase::Armed),
            Phase::Bypassing if now.saturating_sub(since) >= CONNECT_DWELL_MS => {
                if let Err(why) = self.connect() {
                    return Some((Why::RouteLost, why));
                }
                set(self, Phase::Connected);
            }
            Phase::Connected if now.saturating_sub(since) >= MUTE_AFTER_CONNECT_MS => {
                // The connect leaves the kernel's mute as it found it: a muted input needs the true/false pair, an unmuted one is
                // already audible.
                if self.read_adec(adec).and_then(|a| a.mute) == Some(false) {
                    set(self, Phase::Audible);
                } else {
                    if !self.luna_mute(adec, true)
                        || !self.poll(VERIFY_WAIT, |route| {
                            route.read_adec(adec).and_then(|a| a.mute) == Some(true)
                        })
                    {
                        return Some((Why::RouteLost, format!("ADEC{adec} did not mute")));
                    }
                    set(self, Phase::Unmuting);
                }
            }
            Phase::Unmuting if now.saturating_sub(since) >= UNMUTE_AFTER_MUTE_MS => {
                if !self.luna_mute(adec, false)
                    || !self.poll(VERIFY_WAIT, |route| {
                        route.read_adec(adec).and_then(|a| a.mute) == Some(false)
                    })
                {
                    return Some((Why::RouteLost, format!("ADEC{adec} stayed muted")));
                }
                set(self, Phase::Audible);
            }
            _ => {}
        }
        None
    }

    /// Connects the session's ADEC to the main output, journalling the intent first.
    fn connect(&mut self) -> Result<(), String> {
        self.health_gate().map_err(|r| r.reason)?;
        let (adec, base) = {
            let s = self.session.as_ref().ok_or("no session")?;
            (s.journal.adec, sndout::parse_adec(&s.journal.kernel))
        };
        if self
            .sndout
            .as_ref()
            .is_some_and(|s| !s.connected_adecs().is_empty())
        {
            return Err("LG connected an audio input itself".into());
        }
        let now = self
            .read_adec(adec)
            .ok_or_else(|| format!("ADEC{adec} cannot be read"))?;
        if now != base {
            return Err(format!(
                "ADEC{adec} changed since the session was armed ({})",
                now.render()
            ));
        }
        let mut journal = self.session.as_ref().ok_or("no session")?.journal.clone();
        journal.connected = true;
        // A write to flash cannot be cut into slices, so the reading is refreshed on either side of it.
        self.publish_reading();
        self.save_journal(&journal)?;
        self.publish_reading();
        if let Some(s) = &mut self.session {
            s.journal = journal;
        }
        if !self.luna_link("connect", adec) {
            return Err(format!("the audio service refused to connect ADEC{adec}"));
        }
        if !self.poll(VERIFY_WAIT, |route| {
            route.read_sndout().is_some_and(|s| s.adec_connected(adec))
        }) {
            return Err(format!("ADEC{adec} did not show as connected"));
        }
        Ok(())
    }

    // ---- ending ---------------------------------------------------------------------------------------------------------

    /// Puts one session's changes back: silence, the input's mute as it was found, disconnect, words. Every step checks that what
    /// it would undo is still what the session did, nothing is written while the audio stack is wedged, and what the TV looks like
    /// afterwards is compared with what the session found. A helper that is stopping has seconds, not minutes (`hurry`): it does
    /// not wait for the player to close its renderer, so an input found unmuted stays muted, and it skips the comparison. What
    /// cannot be finished stays journalled for the next start.
    fn restore(&mut self, journal: &Journal, owner_alive: bool, hurry: bool) -> Restore {
        if let Some(why) = &self.hung {
            return Restore::Pending(why.clone());
        }
        if let Health::Wedged(why) = health::check(&*self.host) {
            return Restore::Pending(format!(
                "audio control is stuck ({why}); LG's settings stay as they are until it recovers"
            ));
        }
        let adec = journal.adec;
        let base = sndout::parse_adec(&journal.kernel);
        let kernel = self.read_adec(adec);
        // The connection and the source read the same whether LG's input or the session made it, so only `lg_uses` tells them apart.
        // Muting or disconnecting a decoder LG decodes on would cut its live audio.
        let lg = kernel.as_ref().is_some_and(|a| lg_uses(a, &base));
        let ours = kernel.is_some() && !lg;
        if journal.connected && ours && self.read_sndout().is_some_and(|s| s.adec_connected(adec)) {
            // Silence first: the player may still be writing the stream.
            self.luna_mute(adec, true);
            self.poll(VERIFY_WAIT, |route| {
                route.read_adec(adec).and_then(|a| a.mute) == Some(true)
            });
            // An input found unmuted goes back unmuted, but only once the player has closed its renderer: with the stream gone
            // there is nothing left to play, so no noise. Otherwise it stays muted and the audit says so.
            if base.mute == Some(false)
                && !hurry
                && self.poll(RENDERER_CLOSE_WAIT, |route| {
                    route.renderer_closed(journal.pid)
                })
            {
                self.luna_mute(adec, false);
                self.poll(VERIFY_WAIT, |route| {
                    route.read_adec(adec).and_then(|a| a.mute) == Some(false)
                });
            }
            self.luna_link("disconnect", adec);
            let gone = self.poll(VERIFY_WAIT, |route| {
                route.read_sndout().is_some_and(|s| !s.adec_connected(adec))
            });
            if !gone && owner_alive {
                return Restore::Pending(format!("ADEC{adec} will not disconnect"));
            }
            self.host.sleep_ms(SETTLE_MS);
        }
        if journal.applied != journal.before {
            let Some(current) = self.read_sndout().and_then(|s| s.earc) else {
                return Restore::Pending("LG's eARC words cannot be read".into());
            };
            if current == journal.applied {
                if let Err(unapplied) = self.apply_words(journal.before) {
                    return Restore::Pending(unapplied.why);
                }
            }
        }
        let _ = self.host.remove(&self.journal_path);
        if !hurry {
            self.audit(journal, lg);
        }
        Restore::Done
    }

    /// Whether the player no longer holds LG's renderer: it exited or closed it, so nothing more is written into the stream.
    fn renderer_closed(&self, pid: u32) -> bool {
        match self.host.fd_targets(pid) {
            Ok(fds) => !fds.iter().any(|target| target == AREN),
            Err(_) => true,
        }
    }

    /// Compares the TV with what the session found: audiooutputd's connection list, the kernel's connected inputs and the ADEC's own
    /// line must read as before, and LG's input services must have come through. Differences are kept for the page; a wedged audio
    /// task or a silent HDMI input service also holds the next session off. When LG's own input has the decoder (`lg`) its
    /// connection and kernel line are LG's, not differences the route left, so only the note that it was left alone is kept.
    fn audit(&mut self, journal: &Journal, lg: bool) {
        let adec = journal.adec;
        let mut notes = Vec::new();
        if lg {
            notes.push(format!(
                "LG started using ADEC{adec} during the session; it was left as LG has it"
            ));
        } else {
            match health::connections(&*self.host) {
                Some(list) if list.is_empty() => {}
                Some(list) => {
                    notes.push(format!("the audio service still lists {}", list.join(", ")))
                }
                None => notes.push("the audio service's connections cannot be read".to_owned()),
            }
            match self.read_sndout() {
                Some(snd) if snd.connected_adecs().is_empty() => {}
                Some(snd) => notes.push(format!(
                    "the kernel still has ADEC{} connected",
                    snd.connected_adecs().join(", ADEC")
                )),
                None => notes.push("LG's sound status cannot be read".to_owned()),
            }
            let base = sndout::parse_adec(&journal.kernel);
            match self.read_adec(adec) {
                Some(now) if now == base => {}
                Some(now) => notes.push(format!(
                    "ADEC{adec} reads {} but was {}",
                    now.render(),
                    base.render()
                )),
                None => notes.push(format!("ADEC{adec} cannot be read")),
            }
        }
        self.probe_input_services();
        self.after = (!notes.is_empty()).then(|| notes.join("; "));
    }

    /// Looks at LG's input services and keeps what is wrong with them, if anything, until they are healthy again.
    fn probe_input_services(&mut self) {
        self.input_fault = health::input_services(&*self.host)
            .err()
            .map(|why| format!("LG's input services did not come through the last session: {why}"));
    }

    /// What the page shows of the last restore: the differences it found, and what is wrong with LG's input services.
    fn after_note(&self) -> Option<String> {
        let parts: Vec<String> = self
            .after
            .iter()
            .chain(&self.input_fault)
            .cloned()
            .collect();
        (!parts.is_empty()).then(|| parts.join("; "))
    }

    fn retry_pending(&mut self, hurry: bool) {
        let Some((journal, _)) = self.pending.take() else {
            return;
        };
        let alive = self.host.start_time(journal.pid).is_some();
        if let Restore::Pending(note) = self.restore(&journal, alive, hurry) {
            self.pending = Some((journal, note));
        }
    }

    /// Ends the session, whatever the reason: first the client is told (so it stops the stream), then LG's settings go back.
    pub fn end(&mut self, why: Why, detail: &str) {
        let Some(session) = &mut self.session else {
            return;
        };
        session.phase = Phase::Ending;
        session.observed.bypassing = false;
        session.observed.why = detail.to_owned();
        let (journal, pid, start) = (session.journal.clone(), session.pid, session.start);
        self.notify();
        let alive = self.host.start_time(pid) == Some(start);
        let hurry = why == Why::HelperExit;
        if let Restore::Pending(note) = self.restore(&journal, alive, hurry) {
            self.pending = Some((journal, note));
        }
        self.session = None;
        self.lost = (why == Why::RouteLost).then(|| detail.to_owned());
        self.end = Some(End {
            why,
            detail: detail.to_owned(),
        });
        self.notify();
    }

    /// The player ends its session. `Ok(false)` when there was none; an error when another process holds it.
    pub fn release(&mut self, pid: u32) -> Result<bool, String> {
        match &self.session {
            Some(s) if s.pid != pid => Err(format!("process {} holds the route", s.pid)),
            Some(_) => {
                self.end(Why::Released, "the player ended the session");
                Ok(true)
            }
            None => {
                self.lost = None;
                self.notify();
                Ok(false)
            }
        }
    }

    /// Replays a journal an earlier helper left: puts LG's settings back if the boot is the same, drops it otherwise.
    pub fn recover(&mut self) -> Option<String> {
        let text = self.host.read(&self.journal_path).ok()?;
        let journal = Journal::parse(&text).filter(|j| j.boot == self.boot_id());
        let Some(journal) = journal else {
            let _ = self.host.remove(&self.journal_path);
            return Some("dropped a stale journal".into());
        };
        let note = match self.restore(&journal, false, false) {
            Restore::Done => "put LG's audio settings back after a helper restart".to_owned(),
            Restore::Pending(why) => {
                let note = format!("LG's audio settings are not back yet: {why}");
                self.pending = Some((journal, why));
                note
            }
        };
        self.end = Some(End {
            why: Why::Recovered,
            detail: note.clone(),
        });
        self.notify();
        Some(note)
    }

    /// One monitoring step, to run about every [`Route::tick`] milliseconds returned. Handles the lifecycle (plugin off, standby,
    /// helper stopping, player gone), watches a live session, and keeps the readiness reading fresh.
    pub fn tick(&mut self, env: &Env) -> u64 {
        self.env = env.clone();
        let mut ended = false;
        if let Some(s) = &self.session {
            let gone = self.host.start_time(s.pid) != Some(s.start);
            let ending = if !env.enabled {
                Some((Why::Disabled, "the plugin was switched off"))
            } else if env.stopping {
                Some((Why::HelperExit, "the helper is stopping"))
            } else if !env.screen_on {
                Some((Why::Standby, "the TV went to standby"))
            } else if gone {
                Some((Why::OwnerExited, "the player exited"))
            } else {
                None
            };
            let ending = ending
                .map(|(why, detail)| (why, detail.to_owned()))
                .or_else(|| {
                    self.monitoring = true;
                    let ending = self.monitor();
                    self.monitoring = false;
                    ending
                });
            if let Some((why, detail)) = ending {
                self.end(why, &detail);
                ended = true;
            }
        }
        if self.session.is_none() {
            // A restore that has just failed is not tried again in the same breath, and a stopping helper has no time for a slow one.
            if !ended {
                self.retry_pending(env.stopping);
            }
            if env.enabled && !env.stopping {
                self.refresh_idle();
            }
        }
        self.notify();
        if self.session.is_some() {
            ARMED_TICK_MS
        } else {
            IDLE_TICK_MS
        }
    }

    // ---- reporting ------------------------------------------------------------------------------------------------------

    pub fn snapshot(&self) -> Snapshot {
        let env = &self.env;
        // An identity that could not be read is not a refusal: the route waits and looks again.
        let refused = self
            .firmware
            .as_ref()
            .filter(|fw| !fw.unread)
            .and_then(|fw| fw.verdict.as_ref().err());
        let (state, reason) = if !env.enabled {
            (State::Off, "Switched off".to_owned())
        } else if let Some(why) = refused {
            (State::Refused, why.clone())
        } else if let Some(s) = &self.session {
            (State::Armed, status_text(s))
        } else if let Some(why) = &self.lost {
            (State::Lost, why.clone())
        } else if let Err(why) = &self.ready {
            (State::NotReady, why.clone())
        } else {
            (State::Ready, String::new())
        };
        let ready = env.enabled
            && refused.is_none()
            && self.session.is_none()
            && self.pending.is_none()
            && self.ready.is_ok();
        let session = self.session.as_ref().map(|s| SessionView {
            pid: s.pid,
            tail: s.tail.clone(),
            converter: s.converter.clone(),
            adec: s.journal.adec,
            phase: s.phase,
            bypassing: s.observed.bypassing,
            mixed: s.observed.mixed,
            converter_present: s.observed.converter_present,
            connected: s.phase.connected(),
            audible: s.phase == Phase::Audible,
            why: s.observed.why.clone(),
            at: s.observed.at,
        });
        Snapshot {
            state,
            ready,
            reason,
            firmware: self.firmware.clone(),
            earc: self.sndout.as_ref().and_then(|s| s.earc_on),
            words: self
                .sndout
                .as_ref()
                .and_then(|s| s.earc)
                .map(|w| w.to_string()),
            codec: self.sndout.as_ref().and_then(|s| s.arc_codec.clone()),
            output: env.sound_output.clone(),
            digital: env.digital.clone(),
            at: session
                .as_ref()
                .map_or_else(|| self.host.now_ms(), |s| s.at),
            session,
            end: self.end.clone(),
            pending: self.pending.as_ref().map(|(_, note)| note.clone()),
            after: self.after_note(),
        }
    }
}

fn status_text(s: &Session) -> String {
    if !s.observed.why.is_empty() && s.phase != Phase::Armed {
        return s.observed.why.clone();
    }
    match s.phase {
        Phase::Armed => "Waiting for the kernel to report Pass-Through",
        Phase::Bypassing => "Passing the stream through; the output connects shortly",
        Phase::Connected | Phase::Unmuting => "Connecting the output",
        Phase::Audible => "Passthrough is audible",
        Phase::Ending => "Putting LG's settings back",
    }
    .to_owned()
}

/// Whether the kernel's line of the session's decoder shows LG using it. `Connect=` reads the same on the C4's ADEC1 (`HDMI_PORT2`)
/// before, during and after a session, so it cannot tell LG's connection from the route's; `Start=` stays 0 through a session and
/// is 1 while LG's own input decodes on it (`idle_adec` refuses a decoder that is). A changed `Connect=` is LG rewiring it.
fn lg_uses(now: &Adec, base: &Adec) -> bool {
    now.start == Some(true) || now.connect != base.connect
}

#[cfg(test)]
#[path = "route_tests.rs"]
mod tests;
