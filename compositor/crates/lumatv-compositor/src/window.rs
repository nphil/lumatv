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
    /// A type this compositor does not know; kept by name so a TV session shows it.
    Unknown,
}

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
            _ => Unknown,
        }
    }

    /// The stacking layer, bottom to top. Cards share one slot where only the foreground one shows.
    pub fn layer(self) -> Layer {
        use WindowType::*;
        match self {
            Card | Home | Screensaver | Unknown => Layer::Cards,
            Overlay | Floating | Restricted => Layer::Overlays,
            SystemUi | TopUi | Volume | Indicator | Spinner | LongPressSpinner | InfiniteSpinner | CoverUi => Layer::SystemUi,
            Popup | Alert | AdvancedAlert | PinCodePrompt | Toast | BootPopup => Layer::Popups,
            Keyboard | VoiceAgent => Layer::Keyboard,
            ScreenCover | Splash => Layer::Cover,
        }
    }

    /// A card fills the output and is what the app manager treats as the app.
    pub fn is_card(self) -> bool {
        self.layer() == Layer::Cards
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Layer {
    Cards,
    Overlays,
    SystemUi,
    Popups,
    Keyboard,
    Cover,
}

impl Layer {
    pub const ALL: [Layer; 6] = [Layer::Cards, Layer::Overlays, Layer::SystemUi, Layer::Popups, Layer::Keyboard, Layer::Cover];
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
        }
    }

    pub fn app_id(&self) -> &str {
        self.props.get("appId").map(String::as_str).unwrap_or("")
    }

    /// Applies one `set_property`; returns whether the window type changed (the scene must re-layer).
    pub fn set_property(&mut self, name: &str, value: &str) -> bool {
        let mut relayer = false;
        match name {
            "_WEBOS_WINDOW_TYPE" => {
                let k = WindowType::parse(value);
                relayer = k != self.kind;
                self.kind = k;
            }
            "title" => self.title = value.to_string(),
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

    pub fn needs_focus(&self) -> bool {
        self.props.get("needFocus").map(|v| v != "false").unwrap_or(true)
    }
}
