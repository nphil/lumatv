//! A window: a toplevel surface with its `wl_shell_surface` and `wl_webos_shell_surface` state, LG's window
//! type and properties, and where it sits.

use std::collections::BTreeMap;

use crate::surface::SurfaceId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub u32);

/// LG's window types, from `set_property("_WEBOS_WINDOW_TYPE", …)`. The names are the ones LG's compositor QML
/// on the set switches on (`docs/research/compositor-c2-result.md` §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowType {
    Card,
    Home,
    Overlay,
    Floating,
    Popup,
    Restricted,
    TopUi,
    SystemUi,
    Screensaver,
    Keyboard,
    ScreenCover,
    Volume,
    VoiceAgent,
    Toast,
    Splash,
    Spinner,
    PinCodePrompt,
    LongPressSpinner,
    InfiniteSpinner,
    Indicator,
    CoverUi,
    BootPopup,
    Alert,
    AdvancedAlert,
    /// LumaTV's own Home, drawn by the shell client (`shell.rs`) in place of the Home stub's card while that card is the
    /// foreground one: `_WEBOS_WINDOW_TYPE_LUMATV_HOME`, or a window titled [`SHELL_HOME_TITLE`].
    ShellHome,
    /// LumaTV's system layer (the volume display, toasts and alerts, the PIN prompt, the quick panel and the input picker, the
    /// screensaver), above every window: `_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM`, or a window titled [`SHELL_SYSTEM_TITLE`].
    ShellSystem,
    /// A type this compositor does not know; kept by name so a TV session shows it.
    Unknown,
}

/// The titles the shell client gives its two windows, the second way they are told apart from the apps' (a plain `Window`
/// cannot set LG's window type, a `WebOSWindow` can).
pub const SHELL_HOME_TITLE: &str = "LumaTV shell: home";
pub const SHELL_SYSTEM_TITLE: &str = "LumaTV shell: system";

impl WindowType {
    pub fn parse(name: &str) -> WindowType {
        use WindowType::*;
        match name {
            "_WEBOS_WINDOW_TYPE_CARD" => Card,
            "_WEBOS_WINDOW_TYPE_HOME" => Home,
            "_WEBOS_WINDOW_TYPE_OVERLAY" => Overlay,
            "_WEBOS_WINDOW_TYPE_FLOATING" => Floating,
            "_WEBOS_WINDOW_TYPE_POPUP" => Popup,
            "_WEBOS_WINDOW_TYPE_RESTRICTED" => Restricted,
            "_WEBOS_WINDOW_TYPE_TOPUI" => TopUi,
            "_WEBOS_WINDOW_TYPE_SYSTEM_UI" => SystemUi,
            "_WEBOS_WINDOW_TYPE_SCREENSAVER" => Screensaver,
            "_WEBOS_WINDOW_TYPE_KEYBOARD" => Keyboard,
            "_WEBOS_WINDOW_TYPE_SCREENCOVER" => ScreenCover,
            "_WEBOS_WINDOW_TYPE_VOLUME" => Volume,
            "_WEBOS_WINDOW_TYPE_VOICEAGENT" => VoiceAgent,
            "_WEBOS_WINDOW_TYPE_TOAST" => Toast,
            "_WEBOS_WINDOW_TYPE_SPLASH" => Splash,
            "_WEBOS_WINDOW_TYPE_SPINNER" => Spinner,
            "_WEBOS_WINDOW_TYPE_PINCODEPROMPT" => PinCodePrompt,
            "_WEBOS_WINDOW_TYPE_LONGPRESSSPINNER" => LongPressSpinner,
            "_WEBOS_WINDOW_TYPE_INFINITESPINNER" => InfiniteSpinner,
            "_WEBOS_WINDOW_TYPE_INDICATOR" => Indicator,
            "_WEBOS_WINDOW_TYPE_COVERUI" => CoverUi,
            "_WEBOS_WINDOW_TYPE_BOOTPOPUP" => BootPopup,
            "_WEBOS_WINDOW_TYPE_ALERT" => Alert,
            "_WEBOS_WINDOW_TYPE_ADVANCEDALERT" => AdvancedAlert,
            "_WEBOS_WINDOW_TYPE_LUMATV_HOME" => ShellHome,
            "_WEBOS_WINDOW_TYPE_LUMATV_SYSTEM" => ShellSystem,
            _ => Unknown,
        }
    }

    /// The stacking layer, bottom to top. Cards share one slot where only the foreground one shows.
    pub fn layer(self) -> Layer {
        use WindowType::*;
        match self {
            ShellHome => Layer::Home,
            Card | Home | Screensaver | Unknown => Layer::Cards,
            Overlay | Floating | Restricted => Layer::Overlays,
            SystemUi | TopUi | Volume | Indicator | Spinner | LongPressSpinner | InfiniteSpinner | CoverUi => Layer::SystemUi,
            Popup | Alert | AdvancedAlert | PinCodePrompt | Toast | BootPopup => Layer::Popups,
            Keyboard | VoiceAgent => Layer::Keyboard,
            ShellSystem => Layer::Shell,
            ScreenCover | Splash => Layer::Cover,
        }
    }

    /// A card fills the output and is what the app manager treats as the app.
    pub fn is_card(self) -> bool {
        self.layer() == Layer::Cards
    }

    /// One of the shell client's two windows: neither a card nor anything LG's views know.
    pub fn is_shell(self) -> bool {
        matches!(self, WindowType::ShellHome | WindowType::ShellSystem)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    /// LumaTV's Home, under the cards (drawn in the card slot while the Home stub's card is the foreground one).
    Home,
    Cards,
    Overlays,
    SystemUi,
    Popups,
    Keyboard,
    /// LumaTV's system layer.
    Shell,
    Cover,
}

impl Layer {
    pub const ALL: [Layer; 8] = [Layer::Home, Layer::Cards, Layer::Overlays, Layer::SystemUi, Layer::Popups, Layer::Keyboard, Layer::Shell, Layer::Cover];
}

/// `wl_webos_shell_surface.state`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellState {
    Default = 0,
    Minimized = 1,
    Maximized = 2,
    Fullscreen = 3,
}

impl ShellState {
    pub fn from_u32(v: u32) -> ShellState {
        match v {
            1 => ShellState::Minimized,
            2 => ShellState::Maximized,
            3 => ShellState::Fullscreen,
            _ => ShellState::Default,
        }
    }
}

/// `wl_webos_shell_surface.webos_key` bits, the mask an app sets for the keys it wants delivered.
pub mod key_mask {
    pub const HOME: u32 = 1;
    pub const BACK: u32 = 2;
    pub const EXIT: u32 = 4;
    pub const NAVIGATION: u32 = 8 | 16 | 32 | 64 | 128;
    pub const NUMERIC: u32 = 256;
    pub const COLOUR: u32 = 512 | 1024 | 2048 | 4096;
    pub const DEFAULT: u32 = 0x0FFF_FFF8;
}

pub struct Window {
    pub id: WindowId,
    pub surface: SurfaceId,
    pub kind: WindowType,
    pub state: ShellState,
    pub location_hint: u32,
    pub key_mask: u32,
    pub title: String,
    pub class: String,
    /// Every `set_property` as sent, unknown names included.
    pub props: BTreeMap<String, String>,
    /// Top-left in output pixels. Cards are always at the origin; others take `windowPosition`/`set_location_hint`.
    pub position: (i32, i32),
    /// Order of mapping (0 while unmapped): the draw order of what has a buffer.
    pub map_serial: u64,
    /// When the card last came to the front: at its mapping, or at its `set_state(fullscreen)`, which LG's fullscreen
    /// container honours before the card has a buffer (a preloaded app relaunched draws only once SAM has it
    /// foreground, and SAM learns that from the compositor). 0 for a card that never did, or unmapped since.
    pub front: u64,
    pub has_wl_shell: bool,
    pub has_webos_shell: bool,
    /// Whether a commit without a buffer was seen while unmapped (journalled once).
    pub bare_commit_seen: bool,
    /// What the client was last told by `wl_webos_shell_surface.exposed`: its rectangle (true) or none (false). Sent
    /// on each change only, as OSE's `WebOSSurfaceItem::m_exposed` latches it.
    pub exposed: bool,
    /// The shell's system layer has a surface that wants the keys (a panel is open, a prompt is up): it takes the keyboard
    /// focus and the pointer. Off, it is drawn but lets both through to whatever is under it. Set over the UI channel.
    pub takes_keys: bool,
    /// Commits with a new buffer: what the client drew, whether or not it was shown (the shell's idle cost is read off it).
    pub commits: u64,
}

impl Window {
    pub fn new(id: WindowId, surface: SurfaceId) -> Self {
        Window {
            id,
            surface,
            kind: WindowType::Card,
            state: ShellState::Default,
            location_hint: 0,
            key_mask: key_mask::DEFAULT,
            title: String::new(),
            class: String::new(),
            props: BTreeMap::new(),
            position: (0, 0),
            map_serial: 0,
            front: 0,
            has_wl_shell: false,
            has_webos_shell: false,
            bare_commit_seen: false,
            exposed: false,
            takes_keys: false,
            commits: 0,
        }
    }

    pub fn app_id(&self) -> &str {
        self.props.get("appId").map(String::as_str).unwrap_or("")
    }

    /// Applies one `set_property`; returns whether the window type changed (the scene must re-layer). A window the title
    /// made a shell window keeps that through a later default type (a plain Qt `Window` has no type of its own to send).
    pub fn set_property(&mut self, name: &str, value: &str) -> bool {
        let mut relayer = false;
        match name {
            "_WEBOS_WINDOW_TYPE" => {
                let k = WindowType::parse(value);
                if k.is_shell() || !self.kind.is_shell() {
                    relayer = k != self.kind;
                    self.kind = k;
                }
            }
            "title" => {
                self.title = value.to_string();
                let shell = match value {
                    SHELL_HOME_TITLE => Some(WindowType::ShellHome),
                    SHELL_SYSTEM_TITLE => Some(WindowType::ShellSystem),
                    _ => None,
                };
                if let Some(k) = shell {
                    relayer = k != self.kind;
                    self.kind = k;
                }
            }
            "windowPosX" => {
                if let Ok(v) = value.parse() {
                    self.position.0 = v;
                }
            }
            "windowPosY" => {
                if let Ok(v) = value.parse() {
                    self.position.1 = v;
                }
            }
            _ => {}
        }
        self.props.insert(name.to_string(), value.to_string());
        relayer
    }

    pub fn access_policy(&self, name: &str) -> bool {
        self.props.get(name).map(|v| v == "true").unwrap_or(false)
    }

    /// Whether the window takes the keyboard focus when it is on top. The on-screen keyboard never does by default: the field it types
    /// into stays focused (LG's input panel surface refuses the focus on a click as well). The shell's system layer only while it has
    /// something that wants the keys.
    pub fn needs_focus(&self) -> bool {
        match self.kind {
            WindowType::ShellSystem => self.takes_keys,
            _ => self.props.get("needFocus").map(|v| v != "false").unwrap_or(self.kind != WindowType::Keyboard),
        }
    }
}
