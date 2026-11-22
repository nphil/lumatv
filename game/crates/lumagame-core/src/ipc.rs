//! The messages between the app and an engine in a process of its own.
//!
//! One JSON object a line: requests on the engine's standard input, events on its standard output, its log on standard error. Lines of
//! JSON because this channel carries only decisions (a launch, a state, a page of a library), a few a second at most, and has to be
//! readable in a log; media never crosses it. Words (message types, enum values) are kebab-case and field names snake_case.
//!
//! Rules both sides keep:
//! - The app speaks first, with `hello`. The engine answers `ready`, or `error` with `protocol` and exits.
//! - Every request carries `seq`. An event that answers one carries it back as `re`; an event the engine volunteers has none.
//! - An engine answers a request type it does not know with `error` and `unsupported`. The app skips an event type it does not know.
//!   Both ignore fields they do not know, so a field can be added without a new [`PROTOCOL`].
//! - One session and one link attempt at a time per engine. A `launch` while a session is live ends as `failed` with `busy`.
//! - An engine whose standard input closes stops its session and exits. The app closing the pipe, or dying, is therefore the whole
//!   shutdown path, and nothing an engine holds outlives the app.

use crate::engine::{AuthMethod, Codec, Descriptor};
use crate::ids::{ExternalId, ProviderGameId, SessionId, TargetId};
use crate::settings::{SettingsSchema, Values};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// The contract's version. It changes only when a side could no longer make sense of the other: a new required field, a new session
/// state, a changed meaning. An engine's manifest names the one it was built for.
pub const PROTOCOL: u32 = 1;

/// The longest line either side accepts. A library page is the largest message; 200 entries with artwork addresses are about 100 kB.
pub const MAX_LINE_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToEngine {
    pub seq: u64,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FromEngine {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub re: Option<u64>,
    #[serde(flatten)]
    pub event: Event,
}

impl FromEngine {
    pub fn told(event: Event) -> Self {
        Self { re: None, event }
    }

    pub fn answer(seq: u64, event: Event) -> Self {
        Self { re: Some(seq), event }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Request {
    Hello(Hello),
    /// The engine's settings changed. Sent whole; they apply from the next launch.
    Settings {
        values: Values,
    },
    /// Asks for the targets now, and from then on whenever they change.
    Targets,
    /// A host by address, for an engine with `manual-host`.
    AddTarget {
        address: String,
    },
    /// Unlinks a target and deletes what the engine kept for it.
    Forget {
        target: TargetId,
    },
    Wake {
        target: TargetId,
    },
    /// Starts linking: a new target when `target` is absent, else that one again.
    Auth {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<TargetId>,
        method: AuthMethod,
    },
    /// The PIN typed on the TV, for `enter-pin`.
    AuthPin {
        pin: String,
    },
    AuthCancel,
    /// One page of a target's library. `cursor` is the `next` of the page before; absent for the first.
    Library {
        target: TargetId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
    },
    /// Asks whether one game can start on one target right now. Answered with `readiness`.
    Check {
        target: TargetId,
        game: ProviderGameId,
    },
    Launch {
        session: SessionId,
        target: TargetId,
        game: ProviderGameId,
    },
    /// Ends the stream. `end_game` also closes the game on the far side, for an engine with `end-game`.
    Stop {
        session: SessionId,
        end_game: bool,
    },
    /// Who the controller talks to while a session runs: the game, or the app's overlay.
    Route {
        session: SessionId,
        input: InputRoute,
    },
    /// Turns stream measurements on or off. Off by default: an engine measures nothing nobody reads.
    Stats {
        session: SessionId,
        on: bool,
        /// The menu can request ordinary stream metrics while controller timing stays off.
        #[serde(default)]
        pad_timing: bool,
    },
    /// Asks for the legs of a control's round trip the engine can time; answered with `round-trip`. Only for an engine with the
    /// `round-trip` feature.
    RoundTrip {
        session: SessionId,
    },
    /// Opens Steam's overlay over the session's game, for an engine with `steam-overlay`. The app routes the controller to the game
    /// first, so the overlay gets the pad.
    SteamOverlay {
        session: SessionId,
    },
    /// An action field of the settings page was pressed.
    Action {
        key: String,
    },
}

/// The first message: where the engine may keep things and what the TV can show.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub app_version: String,
    /// The engine's own directory, kept across updates. Credentials and pairings live here and nowhere else.
    pub data_dir: String,
    /// May be emptied at any time.
    pub cache_dir: String,
    /// The feeder's socket, for an engine whose video goes through it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feeder: Option<String>,
    /// A BCP 47 tag, `en-US`.
    pub locale: String,
    pub display: Display,
    #[serde(default)]
    pub settings: Values,
}

/// What this TV plays, so an engine never asks its far side for a stream the panel or the decoder cannot take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Display {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub hdr10: bool,
    pub codecs: Vec<Codec>,
}

word_enum! {
    pub enum InputRoute {
        Game => "game",
        /// The engine stops sending input to the game and, with `overlay-keys`, hands controller keys to the app as `key` events.
        App => "app",
    }
    unknown => Other
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event {
    Ready {
        descriptor: Descriptor,
        #[serde(default)]
        schema: SettingsSchema,
    },
    /// Every target the engine knows, whole, each time one changes.
    Targets {
        targets: Vec<Target>,
    },
    Auth {
        #[serde(flatten)]
        state: AuthState,
    },
    Library {
        target: TargetId,
        entries: Vec<LibraryEntry>,
        /// Present while more pages follow.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<String>,
    },
    Readiness {
        target: TargetId,
        game: ProviderGameId,
        readiness: Readiness,
    },
    Session {
        session: SessionId,
        #[serde(flatten)]
        state: SessionState,
    },
    Stats {
        session: SessionId,
        stats: Stats,
    },
    RoundTrip {
        session: SessionId,
        trip: RoundTrip,
    },
    /// A controller key, while the input is routed to the app.
    Key {
        key: PadKey,
        down: bool,
    },
    /// The controller asked for the app's overlay. The app answers with `route`.
    Overlay {
        session: SessionId,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
    /// A type this build does not know, from a newer engine. Skipped.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub id: TargetId,
    /// What the owner calls it: the account's name, the PC's name.
    pub name: String,
    /// A second line: the plan, the address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub link: Link,
    pub reach: Reach,
}

word_enum! {
    pub enum Link {
        Linked => "linked",
        /// Was linked; the far side wants it done again.
        Expired => "expired",
        /// Seen on the network, never linked.
        Found => "found",
    }
    unknown => Other
}

word_enum! {
    pub enum Reach {
        Online => "online",
        /// Off the network, and the engine can wake it.
        Asleep => "asleep",
        Offline => "offline",
    }
    unknown => Unknown
}

word_enum! {
    /// Whether one game can start on one target. Only `ready` is a checked fact; the app never treats the others as one.
    #[derive(Default)]
    pub enum Readiness {
        /// Checked: the target answers, is linked, and has the game.
        Ready => "ready",
        /// The target has to be linked again first.
        SignIn => "sign-in",
        /// The host sleeps and can be woken.
        Asleep => "asleep",
        /// The game is not installed or not owned there.
        Install => "install",
        Offline => "offline",
    }
    unknown => #[default] Unknown
}

impl Readiness {
    pub fn is_ready(self) -> bool {
        self == Self::Ready
    }

    /// Whether a launch can be tried: everything except the two states known to block it.
    pub fn can_attempt(self) -> bool {
        !matches!(self, Self::Install | Self::Offline)
    }
}

word_enum! {
    #[derive(Default)]
    pub enum Role {
        #[default]
        Game => "game",
        /// Something that opens more than one game: a desktop, Steam Big Picture, a store client. Listed, never merged with a game.
        Launcher => "launcher",
    }
    unknown => Other
}

/// One game as one target lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub id: ProviderGameId,
    pub title: String,
    #[serde(default)]
    pub role: Role,
    /// The game's identities that hold everywhere. Left empty when the engine knows none: a title is never one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external: Vec<ExternalId>,
    /// The store this copy is owned in, as a lowercase word (`steam`, `epic`), and the account there when the engine knows it. They
    /// decide what the app may say about saves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_account: Option<String>,
    #[serde(default)]
    pub art: Art,
    #[serde(default)]
    pub readiness: Readiness,
    /// A session of this game is still running on the far side and can be picked up.
    #[serde(default)]
    pub resumable: bool,
}

/// Artwork addresses: `https` URLs, or paths inside the engine's cache directory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Art {
    /// Upright box art, 2:3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    /// A wide backdrop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero: Option<String>,
    /// The title as a transparent image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum AuthState {
    DeviceCode {
        url: String,
        code: String,
        /// What a QR code should hold, when it differs from `url`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        qr: Option<String>,
        expires_s: u32,
    },
    ShowPin {
        pin: String,
    },
    EnterPin {
        digits: u8,
    },
    /// Waiting for the owner to allow this TV on the host. `hint` is what to look for there.
    Approval {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hint: Option<String>,
    },
    Done {
        target: TargetId,
    },
    Failed {
        reason: AuthFail,
        message: String,
    },
}

word_enum! {
    pub enum AuthFail {
        Cancelled => "cancelled",
        Expired => "expired",
        Denied => "denied",
        WrongPin => "wrong-pin",
        Network => "network",
    }
    unknown => Other
}

/// Where a launch stands, as its engine reports it. These are the states the app shows by name; an engine maps its own onto them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum SessionState {
    Checking,
    /// The launch waits for the target to be linked; `auth` events carry the steps, and the launch goes on by itself afterwards.
    SignIn,
    Queue {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        position: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        eta_s: Option<u32>,
    },
    Starting,
    /// The game is on screen and takes input. `stream` is what was agreed with the far side; absent when nothing is streamed.
    Running {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stream: Option<StreamInfo>,
    },
    Ended {
        reason: EndReason,
    },
    Failed {
        reason: FailReason,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamInfo {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub hdr: bool,
    pub audio_channels: u8,
    /// Why a requested HDR picture was delivered as SDR, when the engine reports it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdr_reason: Option<String>,
}

word_enum! {
    pub enum EndReason {
        /// The app asked.
        Stopped => "stopped",
        /// The far side ended it: the game quit, the host closed the stream.
        Remote => "remote",
        /// The stream is closed and the game still runs there, to be picked up later.
        Suspended => "suspended",
    }
    unknown => Other
}

word_enum! {
    pub enum FailReason {
        Auth => "auth",
        Offline => "offline",
        NotInstalled => "not-installed",
        /// The account may not play this: no ownership, or a plan that does not cover it.
        NotEntitled => "not-entitled",
        /// The target already runs a session, or this engine does.
        Busy => "busy",
        Network => "network",
        /// The TV's player refused or lost the stream.
        Video => "video",
        Timeout => "timeout",
        /// The engine itself broke.
        Engine => "engine",
    }
    unknown => Other
}

/// Stream measurements. Each is what its name says, taken where its engine can take it; a value that was not measured is absent,
/// never estimated, and nothing here claims the TV's decoder or panel delay, which no engine can see.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    /// Round trip to the far side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitrate_kbps: Option<u32>,
    /// Frames received a second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f32>,
    /// Frames dropped since the session began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped: Option<u64>,
    /// Datagrams the TV's network stack discarded because the stream's receive buffer was full, since the session began. Absent where
    /// the platform keeps no count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket_drops: Option<u64>,
    /// From a frame's last packet to its handover to the feeder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ms: Option<f32>,
    /// Capture and encode on the far side, when it reports them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_ms: Option<f32>,
    /// How long frames were held to even out their spacing before the decoder, mean over the last second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pacing_ms: Option<f32>,
    /// Spacing between frames as the far side stamped them, median and 95th percentile over the last second: unevenness here
    /// left the far side that way. Absent while its timestamps do not tell frames apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_gap_ms: Option<[f32; 2]>,
    /// The same spacing as the frames arrived, before any pacing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival_gap_ms: Option<[f32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pad: Option<PadStats>,
}

/// Kernel report timing over the last three seconds, with idle pauses excluded from the rate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PadStats {
    pub name: String,
    pub reports: u32,
    pub rate_hz: f32,
    pub p99_us: u32,
    pub bursts: u32,
    pub burst_gap_us: u32,
    /// Age of the last report when this message was sent; a repeated stream update cannot revive an idle pad.
    pub age_ms: u64,
}

/// The legs of a control's round trip that an engine can time, in milliseconds. As in [`Stats`], a leg that was not measured is absent;
/// none covers the TV's decoder or panel, which only a camera on the screen sees.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RoundTrip {
    /// Presses timed from the kernel's stamp to the engine handing them to its transport, and their median and 99th percentile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presses: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub press_p50_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub press_p99_ms: Option<f32>,
    /// The transport's round trip to the far side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_ms: Option<f32>,
    /// Capture and encode on the far side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_ms: Option<f32>,
    /// Capture on the far side to the frame's arrival here, median and 95th percentile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival_p50_ms: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival_p95_ms: Option<f32>,
    /// From a frame's last packet to its handover to the feeder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_ms: Option<f32>,
}

word_enum! {
    pub enum PadKey {
        Up => "up",
        Down => "down",
        Left => "left",
        Right => "right",
        Ok => "ok",
        Back => "back",
        Menu => "menu",
        PageLeft => "page-left",
        PageRight => "page-right",
    }
    unknown => Other
}

word_enum! {
    pub enum ErrorCode {
        /// The two sides do not speak the same contract.
        Protocol => "protocol",
        /// A request this engine does not have.
        Unsupported => "unsupported",
        /// A request it has, with something in it that it cannot use.
        BadRequest => "bad-request",
        Internal => "internal",
    }
    unknown => Other
}

/// One message as its line, newline included.
pub fn encode<T: Serialize>(message: &T) -> String {
    // The message types hold only strings, numbers and maps keyed by strings, which always serialise.
    let mut line = serde_json::to_string(message).expect("an IPC message serialises");
    line.push('\n');
    line
}

pub fn decode<T: DeserializeOwned>(line: &str) -> Result<T, String> {
    serde_json::from_str(line).map_err(|e| e.to_string())
}

/// The longest failure code diagnostics carry.
pub const FAILURE_CODE_MAX: usize = 24;

/// A failure's message with the engine's own error code appended, as `message (code some_code)`. The code is reduced to lowercase
/// letters, digits and `_` and cut at [`FAILURE_CODE_MAX`], so it is safe to log; the message is shown to the owner as it is.
pub fn failure_text(message: &str, code: &str) -> String {
    let code: String = code.chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_').take(FAILURE_CODE_MAX).collect();
    if code.is_empty() { message.to_owned() } else { format!("{message} (code {code})") }
}

/// The code [`failure_text`] appended, if the message ends with one. Nothing else in a message is ever read as a code.
pub fn failure_code(message: &str) -> Option<&str> {
    let code = message.strip_suffix(')')?.rsplit_once(" (code ")?.1;
    (!code.is_empty() && code.len() <= FAILURE_CODE_MAX && code.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')).then_some(code)
}

/// Cuts a byte stream into lines. Bounded, because the other side is another program and maybe another build: a line that never ends
/// must not grow this side's memory. A line over [`MAX_LINE_BYTES`], or one that is not UTF-8, is dropped whole and counted, and the
/// stream stays usable after it.
#[derive(Debug, Default)]
pub struct LineBuffer {
    partial: Vec<u8>,
    /// Inside a line that already went over the cap: everything up to its newline is thrown away.
    skipping: bool,
    pub dropped: u64,
}

impl LineBuffer {
    pub fn push(&mut self, mut bytes: &[u8], mut line: impl FnMut(&str)) {
        while let Some(at) = bytes.iter().position(|b| *b == b'\n') {
            let head = &bytes[..at];
            bytes = &bytes[at + 1..];
            if std::mem::take(&mut self.skipping) {
                continue;
            }
            if self.partial.len() + head.len() > MAX_LINE_BYTES {
                self.dropped += 1;
            } else {
                self.partial.extend_from_slice(head);
                match std::str::from_utf8(&self.partial) {
                    Ok(text) if text.trim().is_empty() => {}
                    Ok(text) => line(text.trim()),
                    Err(_) => self.dropped += 1,
                }
            }
            self.partial.clear();
        }
        if self.skipping {
            return;
        }
        if self.partial.len() + bytes.len() > MAX_LINE_BYTES {
            self.dropped += 1;
            self.skipping = true;
            self.partial.clear();
        } else {
            self.partial.extend_from_slice(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Capabilities, EngineKind, Feature, Presentation, TargetKind};
    use crate::ids::EngineId;
    use crate::settings::Value;
    use std::collections::BTreeSet;

    fn target() -> TargetId {
        TargetId::new("a1b2").unwrap()
    }

    fn game() -> ProviderGameId {
        ProviderGameId::new("steam:1245620").unwrap()
    }

    fn entry() -> LibraryEntry {
        LibraryEntry {
            id: game(),
            title: "Elden Ring".into(),
            role: Role::Game,
            external: vec!["steam:1245620".parse().unwrap()],
            store: Some("steam".into()),
            store_account: None,
            art: Art { cover: Some("https://example.invalid/cover.jpg".into()), ..Art::default() },
            readiness: Readiness::Ready,
            resumable: false,
        }
    }

    #[test]
    fn the_wire_form_is_one_flat_object_a_line() {
        let launch = ToEngine { seq: 7, request: Request::Launch { session: SessionId(3), target: target(), game: game() } };
        assert_eq!(encode(&launch), "{\"seq\":7,\"type\":\"launch\",\"session\":3,\"target\":\"a1b2\",\"game\":\"steam:1245620\"}\n");

        let queue = FromEngine::told(Event::Session { session: SessionId(3), state: SessionState::Queue { position: Some(4), eta_s: None } });
        assert_eq!(encode(&queue), "{\"type\":\"session\",\"session\":3,\"state\":\"queue\",\"position\":4}\n");

        let code = FromEngine::answer(
            2,
            Event::Auth { state: AuthState::DeviceCode { url: "https://example.invalid/link".into(), code: "ABCD".into(), qr: None, expires_s: 600 } },
        );
        assert_eq!(
            encode(&code),
            "{\"re\":2,\"type\":\"auth\",\"state\":\"device-code\",\"url\":\"https://example.invalid/link\",\"code\":\"ABCD\",\"expires_s\":600}\n"
        );

        assert_eq!(encode(&ToEngine { seq: 1, request: Request::Targets }), "{\"seq\":1,\"type\":\"targets\"}\n");
        assert_eq!(
            encode(&FromEngine::told(Event::Stats { session: SessionId(3), stats: Stats { rtt_ms: Some(8.5), ..Stats::default() } })),
            "{\"type\":\"stats\",\"session\":3,\"stats\":{\"rtt_ms\":8.5}}\n"
        );
        assert_eq!(
            encode(&FromEngine::answer(4, Event::RoundTrip { session: SessionId(3), trip: RoundTrip { network_ms: Some(9.0), ..RoundTrip::default() } })),
            "{\"re\":4,\"type\":\"round-trip\",\"session\":3,\"trip\":{\"network_ms\":9.0}}\n"
        );
    }

    #[test]
    fn every_request_and_event_reads_back_as_it_was_written() {
        let hello = Hello {
            protocol: PROTOCOL,
            app_version: "0.1.0".into(),
            data_dir: "/data/engines/gfn".into(),
            cache_dir: "/data/cache/gfn".into(),
            feeder: Some("/tmp/org.nphil.lumagame/feed.sock".into()),
            locale: "en-US".into(),
            display: Display { width: 3840, height: 2160, refresh_hz: 120, hdr10: true, codecs: vec![Codec::H265, Codec::Av1] },
            settings: Values::from([("hdr".into(), Value::Bool(true)), ("bitrate-mbps".into(), Value::Int(75)), ("tier".into(), Value::Text("uhd120".into()))]),
        };
        let requests = [
            Request::Hello(hello),
            Request::Settings { values: Values::new() },
            Request::Targets,
            Request::AddTarget { address: "192.168.1.20".into() },
            Request::Forget { target: target() },
            Request::Wake { target: target() },
            Request::Auth { target: None, method: AuthMethod::DeviceCode },
            Request::Auth { target: Some(target()), method: AuthMethod::EnterPin },
            Request::AuthPin { pin: "1234".into() },
            Request::AuthCancel,
            Request::Library { target: target(), cursor: Some("p2".into()) },
            Request::Check { target: target(), game: game() },
            Request::Launch { session: SessionId(1), target: target(), game: game() },
            Request::Stop { session: SessionId(1), end_game: true },
            Request::Route { session: SessionId(1), input: InputRoute::App },
            Request::Stats { session: SessionId(1), on: true, pad_timing: true },
            Request::RoundTrip { session: SessionId(1) },
            Request::SteamOverlay { session: SessionId(1) },
            Request::Action { key: "sign-out".into() },
        ];
        for (seq, request) in requests.into_iter().enumerate() {
            let message = ToEngine { seq: seq as u64, request };
            assert_eq!(decode::<ToEngine>(&encode(&message)).unwrap(), message);
        }
        let mut legacy = serde_json::to_value(Request::Stats { session: SessionId(1), on: true, pad_timing: true }).unwrap();
        legacy.as_object_mut().unwrap().remove("pad_timing");
        assert_eq!(serde_json::from_value::<Request>(legacy).unwrap(), Request::Stats { session: SessionId(1), on: true, pad_timing: false });

        let descriptor = Descriptor {
            id: EngineId::new("punktfunk").unwrap(),
            version: "0.1.0".into(),
            capabilities: Capabilities {
                kind: EngineKind::Host,
                targets: TargetKind::Host,
                auth: vec![AuthMethod::EnterPin, AuthMethod::Approval],
                presentation: Presentation::Feeder,
                features: BTreeSet::from([Feature::Discovery, Feature::Wake]),
                video: None,
            },
        };
        let stream = StreamInfo { codec: Codec::H265, width: 3840, height: 2160, fps: 120, hdr: true, audio_channels: 6, sdr_reason: None };
        let session = |state| Event::Session { session: SessionId(1), state };
        let auth = |state| Event::Auth { state };
        let events = [
            Event::Ready { descriptor, schema: SettingsSchema::default() },
            Event::Targets {
                targets: vec![Target { id: target(), name: "Gaming PC".into(), detail: Some("192.168.1.20".into()), link: Link::Linked, reach: Reach::Asleep }],
            },
            auth(AuthState::ShowPin { pin: "4821".into() }),
            auth(AuthState::EnterPin { digits: 4 }),
            auth(AuthState::Approval { hint: None }),
            auth(AuthState::Done { target: target() }),
            auth(AuthState::Failed { reason: AuthFail::WrongPin, message: "That PIN did not match.".into() }),
            Event::Library { target: target(), entries: vec![entry()], next: Some("p2".into()) },
            Event::Readiness { target: target(), game: game(), readiness: Readiness::Install },
            session(SessionState::Checking),
            session(SessionState::SignIn),
            session(SessionState::Queue { position: None, eta_s: Some(90) }),
            session(SessionState::Starting),
            session(SessionState::Running { stream: Some(stream) }),
            session(SessionState::Running { stream: None }),
            session(SessionState::Ended { reason: EndReason::Suspended }),
            session(SessionState::Failed { reason: FailReason::Video, message: "The TV refused the stream.".into() }),
            Event::Stats { session: SessionId(1), stats: Stats { rtt_ms: Some(8.5), dropped: Some(2), socket_drops: Some(3), ..Stats::default() } },
            Event::RoundTrip {
                session: SessionId(1),
                trip: RoundTrip { presses: Some(12), press_p50_ms: Some(0.4), arrival_p95_ms: Some(21.5), ..RoundTrip::default() },
            },
            Event::Key { key: PadKey::Back, down: true },
            Event::Overlay { session: SessionId(1) },
            Event::Error { code: ErrorCode::Unsupported, message: "wake".into() },
        ];
        for event in events {
            let message = FromEngine::answer(9, event);
            assert_eq!(decode::<FromEngine>(&encode(&message)).unwrap(), message);
        }
    }

    #[test]
    fn stream_sdr_reason_is_optional_and_round_trips() {
        let line = r#"{"codec":"h265","width":3840,"height":2160,"fps":60,"hdr":false,"audio_channels":2}"#;
        let mut stream: StreamInfo = decode(line).unwrap();
        assert_eq!(stream.sdr_reason, None);
        assert!(!encode(&stream).contains("sdr_reason"));
        stream.sdr_reason = Some("GeForce NOW sent no HDR".into());
        assert_eq!(decode::<StreamInfo>(&encode(&stream)).unwrap(), stream);
    }

    #[test]
    fn a_newer_engine_does_not_break_an_older_app() {
        assert_eq!(decode::<FromEngine>(r#"{"type":"achievement","name":"First win"}"#).unwrap().event, Event::Unknown);

        let event = decode::<FromEngine>(r#"{"type":"session","session":1,"state":"failed","reason":"region-locked","message":"Not here.","retry_s":30}"#)
            .unwrap()
            .event;
        assert_eq!(event, Event::Session { session: SessionId(1), state: SessionState::Failed { reason: FailReason::Other, message: "Not here.".into() } });

        let page = r#"{"type":"library","target":"a1b2","entries":[{"id":"7","title":"Desktop","role":"tool","genres":["none"]}]}"#;
        let Event::Library { entries, next, .. } = decode::<FromEngine>(page).unwrap().event else { panic!("a library page") };
        assert_eq!((entries[0].role, entries[0].readiness, entries[0].external.len(), next), (Role::Other, Readiness::Unknown, 0, None));

        assert!(decode::<FromEngine>(r#"{"type":"session","session":1,"state":"paused"}"#).is_err(), "a state the app cannot act on is refused, not guessed");
        assert!(decode::<ToEngine>(r#"{"seq":1,"type":"rewind"}"#).is_err(), "which an engine answers with unsupported");
        assert!(decode::<FromEngine>(r#"{"type":"library","target":"a|b","entries":[]}"#).is_err(), "ids are checked as they are read");
    }

    #[test]
    fn readiness_separates_checked_from_possible_from_blocked() {
        let attemptable: Vec<Readiness> = [Readiness::Ready, Readiness::SignIn, Readiness::Asleep, Readiness::Unknown, Readiness::Install, Readiness::Offline]
            .into_iter()
            .filter(|r| r.can_attempt())
            .collect();
        assert_eq!(attemptable, [Readiness::Ready, Readiness::SignIn, Readiness::Asleep, Readiness::Unknown]);
        assert!(Readiness::Ready.is_ready() && !Readiness::Unknown.is_ready() && !Readiness::Asleep.is_ready());
        assert_eq!(serde_json::from_str::<Readiness>("\"queued\"").unwrap(), Readiness::Unknown);
    }

    #[test]
    fn a_line_buffer_cuts_a_stream_into_lines_and_drops_one_that_never_ends() {
        let mut lines = Vec::new();
        let mut buffer = LineBuffer::default();
        let mut push = |buffer: &mut LineBuffer, bytes: &[u8]| buffer.push(bytes, |l| lines.push(l.to_owned()));

        push(&mut buffer, b"{\"a\":1}\n{\"b\"");
        push(&mut buffer, b":2}\r\n\n{\"c\":3}");
        push(&mut buffer, b"\n\xff\xfe\n");
        assert_eq!(buffer.dropped, 1, "the line that is not text");

        let long = vec![b'x'; MAX_LINE_BYTES / 2 + 1];
        push(&mut buffer, &long);
        push(&mut buffer, &long);
        push(&mut buffer, &long);
        push(&mut buffer, b"tail of the long line\n{\"d\":4}\n");
        assert_eq!(buffer.dropped, 2);
        assert!(buffer.partial.is_empty());

        let whole = [vec![b'y'; MAX_LINE_BYTES + 1], b"\n{\"e\":5}\n".to_vec()].concat();
        push(&mut buffer, &whole);
        assert_eq!(buffer.dropped, 3);
        assert_eq!(lines, ["{\"a\":1}", "{\"b\":2}", "{\"c\":3}", "{\"d\":4}", "{\"e\":5}"]);
    }

    #[test]
    fn a_failure_code_survives_the_message_and_only_a_safe_one_is_read_back() {
        let text = failure_text("It failed.", "session_update_busy");
        assert_eq!(text, "It failed. (code session_update_busy)");
        assert_eq!(failure_code(&text), Some("session_update_busy"));
        assert_eq!(failure_text("It failed.", "https://x/?token=a"), "It failed. (code httpsxtokena)");
        assert_eq!(failure_text("It failed.", "!!"), "It failed.");
        for message in ["It failed.", "It failed (code Bad Code)", "It failed (code ", "(code abc)", &format!("x (code {})", "a".repeat(25))] {
            assert_eq!(failure_code(message), None, "{message}");
        }
    }
}
