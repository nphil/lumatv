//! The shared vocabulary of the AI-access module: permission groups and levels, the typed tool calls, and the traits that
//! separate the module's decisions from the things it acts on (the TV, the receiver, the remote's keys, the MCP transport).
//!
//! Nothing here decides anything. The decisions are in `broker` (who may do what), `gate` (what only a physical remote
//! press can approve) and `store` (who is paired); everything an assistant can reach is a variant of [`Call`], so a tool that
//! is not in this file does not exist.

use serde_json::Value;
use std::net::IpAddr;
use std::time::{Duration, Instant};

// ------------------------------------------------------------------------------------------------ permissions

/// What an assistant may do is granted by group, never by single tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    /// See what is playing, power and input state.
    State,
    /// Play, pause, volume, mute.
    Playback,
    /// Open apps, switch inputs.
    Launch,
    /// Turn the TV off.
    Power,
}

impl Group {
    pub const ALL: [Group; 4] = [Group::State, Group::Playback, Group::Launch, Group::Power];

    /// The key used in settings, JSON and logs.
    pub fn id(self) -> &'static str {
        match self {
            Self::State => "state",
            Self::Playback => "playback",
            Self::Launch => "launch",
            Self::Power => "power",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.id() == id)
    }

    /// The words on the TV's AI access page (the plan's table).
    pub fn title(self) -> &'static str {
        match self {
            Self::State => "See what is playing, power and input state",
            Self::Playback => "Play, pause, volume, mute",
            Self::Launch => "Open apps, switch inputs",
            Self::Power => "Turn the TV off",
        }
    }

    /// What a newly paired assistant gets: the plan's table.
    pub fn default_level(self) -> Level {
        match self {
            Self::State | Self::Playback => Level::Always,
            Self::Launch | Self::Power => Level::Ask,
        }
    }
}

/// How much of a group an assistant has. Ordered by how much it allows, so a change is a *widening* when the new level is
/// greater than the old one: widening needs a physical remote press, narrowing never does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    Never,
    /// Every call opens a prompt on the TV; only a physical remote answers it.
    Ask,
    Always,
}

impl Level {
    pub fn id(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Ask => "ask",
            Self::Always => "always",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        [Self::Never, Self::Ask, Self::Always]
            .into_iter()
            .find(|l| l.id() == id)
    }

    /// The words shown on the TV.
    pub fn title(self) -> &'static str {
        match self {
            Self::Never => "Never",
            Self::Ask => "Ask on TV",
            Self::Always => "Always",
        }
    }
}

// ------------------------------------------------------------------------------------------------ tool calls

/// The transport keys an assistant may press. Navigation keys (arrows, OK, Back, Home, Menu) are deliberately not here: an
/// assistant must not drive LG's screens or LumaTV's own menus, least of all the prompt that asks about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKey {
    Play,
    Pause,
    Stop,
    Rewind,
    FastForward,
}

impl MediaKey {
    pub const ALL: [MediaKey; 5] = [
        Self::Play,
        Self::Pause,
        Self::Stop,
        Self::Rewind,
        Self::FastForward,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Stop => "stop",
            Self::Rewind => "rewind",
            Self::FastForward => "fast_forward",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == id)
    }
}

/// One validated call. The arguments are already checked (ranges, enumerations, identifier shapes) when one exists.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    GetPower,
    GetForegroundApp,
    ListInputs,
    ListApps,
    GetNowPlaying,
    GetReceiver,
    /// On the receiver's 0 to 98 scale (what its panel shows in Absolute mode).
    SetVolume {
        level: f64,
    },
    ChangeVolume {
        up: bool,
        steps: u8,
    },
    SetMute {
        muted: bool,
    },
    MediaKey {
        key: MediaKey,
    },
    LaunchApp {
        app_id: String,
    },
    SwitchInput {
        input_id: String,
    },
    Standby,
}

impl Call {
    pub fn group(&self) -> Group {
        match self {
            Self::GetPower
            | Self::GetForegroundApp
            | Self::ListInputs
            | Self::ListApps
            | Self::GetNowPlaying
            | Self::GetReceiver => Group::State,
            Self::SetVolume { .. }
            | Self::ChangeVolume { .. }
            | Self::SetMute { .. }
            | Self::MediaKey { .. } => Group::Playback,
            Self::LaunchApp { .. } | Self::SwitchInput { .. } => Group::Launch,
            Self::Standby => Group::Power,
        }
    }

    /// The tool's name in MCP.
    pub fn tool_name(&self) -> &'static str {
        match self {
            Self::GetPower => "tv_get_power",
            Self::GetForegroundApp => "tv_get_foreground_app",
            Self::ListInputs => "tv_list_inputs",
            Self::ListApps => "tv_list_apps",
            Self::GetNowPlaying => "tv_get_now_playing",
            Self::GetReceiver => "receiver_get_status",
            Self::SetVolume { .. } => "receiver_set_volume",
            Self::ChangeVolume { .. } => "receiver_change_volume",
            Self::SetMute { .. } => "receiver_set_mute",
            Self::MediaKey { .. } => "tv_media_key",
            Self::LaunchApp { .. } => "tv_launch_app",
            Self::SwitchInput { .. } => "tv_switch_input",
            Self::Standby => "tv_standby",
        }
    }

    /// Whether the call changes nothing on the TV or the receiver.
    pub fn is_read_only(&self) -> bool {
        self.group() == Group::State
    }
}

// ------------------------------------------------------------------------------------------------ what is acted on

/// Why something the module asked of the TV or the receiver did not happen. `code` is a short stable word
/// (`unavailable`, `not_found`, `refused`, `timeout`, `failed`); `message` is one plain line safe to show an assistant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionError {
    pub code: &'static str,
    pub message: String,
}

impl ActionError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ActionError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppInfo {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForegroundApp {
    /// `None` when nothing is in front (or the TV does not say).
    pub app_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputInfo {
    /// `HDMI_1`, `HDMI_2`, ...
    pub id: String,
    pub label: String,
    pub connected: bool,
    /// The app that shows this input (`com.webos.app.hdmi2`).
    pub app_id: Option<String>,
}

/// What the module does to the TV. Implemented over `luna-send` and the input device that carries network keys.
pub trait TvControl: Send + Sync {
    fn foreground_app(&self) -> Result<ForegroundApp, ActionError>;
    /// The apps an assistant may open: visible, not hidden, and not on the deny list (LG's settings, the Homebrew tools, LumaTV).
    fn launchable_apps(&self) -> Result<Vec<AppInfo>, ActionError>;
    fn inputs(&self) -> Result<Vec<InputInfo>, ActionError>;
    /// Opens an app. The id is checked against [`TvControl::launchable_apps`] again here, whatever the caller did.
    fn launch_app(&self, app_id: &str) -> Result<AppInfo, ActionError>;
    /// Switches to an input that exists and is connected. Checked against [`TvControl::inputs`] again here.
    fn switch_input(&self, input_id: &str) -> Result<InputInfo, ActionError>;
    /// Puts the TV in standby (the screen goes dark; the set stays reachable).
    fn standby(&self) -> Result<(), ActionError>;
    fn media_key(&self, key: MediaKey) -> Result<(), ActionError>;
}

/// What the module does to the receiver.
pub trait ReceiverControl: Send + Sync {
    /// Sets the volume on the receiver's 0 to 98 scale. The caller has already applied the volume limit.
    fn set_volume(&self, level: f64) -> Result<(), ActionError>;
    fn set_mute(&self, muted: bool) -> Result<(), ActionError>;
}

// ------------------------------------------------------------------------------------------------ the remote's keys

/// One key event read from an input device. `value` is 1 for a press, 0 for a release, 2 for auto-repeat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    /// The device's name as the kernel reports it (`LGE RCU`, `Smart Remote RCU Input`).
    pub device: String,
    pub code: u16,
    pub value: i32,
    pub at: Instant,
}

/// Where key events come from. The real source reads the TV's evdev devices; a test gives a script.
pub trait KeySource: Send + Sync {
    /// Starts listening now: devices are opened here and released when the session is dropped, so nothing is held while no
    /// prompt is open.
    fn open(&self) -> Box<dyn KeySession>;
}

pub trait KeySession: Send {
    /// The next key event from any device the source can see, waiting at most `timeout`; `None` on timeout. A session that
    /// could open no device just times out (a prompt nobody can answer expires: it fails closed).
    fn next(&mut self, timeout: Duration) -> Option<KeyEvent>;
}

/// Sends one key press (down, a short hold, up) to the input device that carries network keys.
pub trait KeyInjector: Send + Sync {
    fn tap(&self, code: u16) -> Result<(), ActionError>;
}

// ------------------------------------------------------------------------------------------------ the transport

/// The assistant a request belongs to, as the transport needs to know it. The name is the assistant's own word for itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantView {
    pub id: String,
    pub name: String,
}

/// A tool as MCP lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: String,
    /// A JSON Schema object (`type: object`, `properties`, `required`, `additionalProperties: false`).
    pub input_schema: Value,
    pub read_only: bool,
}

/// What a tool call came to.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolOutcome {
    /// `text` is one plain sentence; `data` is the structured result (an object).
    Ok { text: String, data: Value },
    /// A refusal or a failure the assistant can read: `code` is a stable word (`denied`, `paused`, `never`, `busy`,
    /// `invalid_arguments`, `not_found`, ...), `message` one plain line.
    Err { code: &'static str, message: String },
}

/// The tools of one module, as the MCP layer sees them.
pub trait ToolHost: Send + Sync {
    /// The tools this assistant can see: those whose group it has not been given `Never` for.
    fn list_tools(&self, who: &AssistantView) -> Vec<ToolDef>;
    /// Runs one tool for one assistant: permission, prompt, action and audit all happen in here.
    fn call_tool(
        &self,
        who: &AssistantView,
        peer: IpAddr,
        name: &str,
        arguments: &Value,
    ) -> ToolOutcome;
}

/// Why a bearer token was not accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthFailure {
    /// No such token (never paired, revoked, or locked out).
    Unknown,
}

/// What a successful pairing hands the assistant, once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paired {
    pub assistant_id: String,
    /// The bearer token. It exists in the clear only in this value and in the assistant's hands; the TV keeps its hash.
    pub token: String,
}

/// Why a pairing attempt failed. The texts are what the assistant is told: they never say whether a code exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PairFailure {
    /// No pairing window is open, the code is wrong, or the window is spent. One answer for all three.
    Refused,
    /// Too many attempts from anywhere lately.
    TooManyAttempts,
    /// The name is empty after cleaning.
    BadName,
}

/// What the MCP layer needs from the module.
pub trait Authenticator: Send + Sync {
    fn authenticate(&self, bearer: &str, peer: IpAddr) -> Result<AssistantView, AuthFailure>;
    fn pair(&self, code: &str, name: &str, peer: IpAddr) -> Result<Paired, PairFailure>;
}
