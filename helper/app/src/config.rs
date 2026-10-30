//! The helper's own configuration (`<data dir>/helper.json`) and where its files live.
//!
//! Two kinds of setting exist and they are kept apart. What a person chooses (weather zip, news edition, the receiver's
//! address) is a *setting*: it has a schema, is validated, and lives in the settings store. This file holds how the process
//! itself runs: the listen port, the log, the paths of programs it calls, and the receiver polling rhythm. A file that does not
//! exist means the defaults; a file that does not parse stops the helper with the reason (a typo in `listen` must not be
//! silently ignored).

use serde::Deserialize;
use std::fmt;
use std::net::IpAddr;
use std::path::{Path, PathBuf};

/// The default loopback port of the API.
pub const DEFAULT_PORT: u16 = 18731;

/// Where the helper's files are, all under the data dir.
#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
}

impl Paths {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            data_dir: data_dir.into(),
        }
    }

    /// `helper.json`
    pub fn config_file(&self) -> PathBuf {
        self.data_dir.join("helper.json")
    }

    /// `log-helper`
    pub fn log_file(&self) -> PathBuf {
        self.data_dir.join("log-helper")
    }

    /// `helper.lock`: held (flock) by the running helper, which writes its pid into it.
    pub fn lock_file(&self) -> PathBuf {
        self.data_dir.join("helper.lock")
    }

    /// `cache/`: the saved-first weather and news documents.
    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir.join("cache")
    }

    /// `settings/`: one file of stored values per settings owner.
    pub fn settings_dir(&self) -> PathBuf {
        self.data_dir.join("settings")
    }

    /// `preload.json`: the apps the owner keeps from preloading (written by `lumatv preload`, read by the helper).
    pub fn preload_file(&self) -> PathBuf {
        self.data_dir.join("preload.json")
    }

    /// `plugins/`: `enabled.json` (which plugins are on) and one directory per installed plugin, with its `plugin.json`.
    pub fn plugins_dir(&self) -> PathBuf {
        self.data_dir.join("plugins")
    }

    /// `ha/link.json`: the address and token of the Home Assistant the plugins' lease dials (`ha_store`; written by `ha-link`).
    pub fn ha_link_file(&self) -> PathBuf {
        self.data_dir.join("ha").join("link.json")
    }

    /// `scrypted/link.json`: the bridge's address, certificate pin, the TV's own token and its device name (`scrypted::store`; written by
    /// `scrypted-link`, and the only thing that switches the cameras on).
    pub fn scrypted_link_file(&self) -> PathBuf {
        self.data_dir.join("scrypted").join("link.json")
    }
}

/// A config problem, in words.
#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    pub listen: Listen,
    pub log: LogConfig,
    pub screen: ScreenConfig,
    pub receiver: ReceiverTuning,
    pub fetch: FetchConfig,
    pub luna: LunaConfig,
    /// The panel's rate after 25 or 50 fps video (`framerate.rs`): put back to 120 Hz on Home.
    pub frame_rate: FrameRateConfig,
    /// The AI-access module (the MCP server). Off until the TV's owner turns it on with the remote; these are only how it runs.
    pub ai: AiConfig,
    /// The plugin broker. It exists only while `plugins/enabled.json` lists a plugin; these are only how it runs.
    pub plugins: PluginsConfig,
    /// What the device is, told to plugins in `welcome` and served at `/v1/device` (the TV by default; see `device.rs`).
    pub device: DeviceConfig,
    /// Where the ambient lights are (`ambient.rs`). Nothing is ever sent to a device that is not named here.
    pub ambient: AmbientConfig,
    /// Voice search (`voice.rs`): where the microphone's audio comes from. Which Home Assistant it goes to is the saved link, not here.
    pub voice: VoiceConfig,
    /// Pictures decoded for the shell (`images.rs`): which folders it may ask for, and the decoder that does the work.
    pub images: ImagesConfig,
    /// Cameras and their event pop-ups through Scrypted's LumaShow bridge (`scrypted/`). Off until the TV is linked (`scrypted-link`):
    /// with no link nothing connects and no picture is ever fetched; these are only tuning.
    pub cameras: CamerasConfig,
    /// Screenshots and screen recordings of the picture LumaTV's compositor composes (`capture.rs`, `compositor/.../screencap`). Off by
    /// default: with `enabled` false the routes refuse and the compositor does nothing; both read this one switch.
    pub capture: CaptureConfig,
    /// The display lab (`displaylab/`): the shell's guided look at the display chip's sharpener, dimming and window. On by default (it can
    /// write only its own short list, and puts everything back); with `enabled` false the routes refuse. A journal an earlier run left is put
    /// back at the helper's start whatever this says.
    pub display_lab: DisplayLabConfig,
    /// How far the helper's scheduling priority is lowered (`nice`): it is background work and must never take a core from the
    /// compositor. 0 leaves it as started.
    pub nice: i32,
    /// Browser origins allowed to call the API (empty: none). Requests from the compositor's QML and from `curl` carry no
    /// `Origin` header and are always allowed; a web page does carry one, and a page on the TV's browser must not be able to
    /// drive the helper.
    pub allowed_origins: Vec<String>,
    /// The brand's app id (from `brand.json`, set by the command line, never read from this file): LumaTV's own app, which an
    /// assistant must never be able to open.
    #[serde(skip)]
    pub brand_app_id: Option<String>,
    /// The brand's QML module (from `brand.json`, set by the command line): where the packaged `art` folder is.
    #[serde(skip)]
    pub brand_qml_module: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: Listen::default(),
            log: LogConfig::default(),
            screen: ScreenConfig::default(),
            receiver: ReceiverTuning::default(),
            fetch: FetchConfig::default(),
            luna: LunaConfig::default(),
            frame_rate: FrameRateConfig::default(),
            ai: AiConfig::default(),
            plugins: PluginsConfig::default(),
            device: DeviceConfig::default(),
            ambient: AmbientConfig::default(),
            voice: VoiceConfig::default(),
            images: ImagesConfig::default(),
            cameras: CamerasConfig::default(),
            capture: CaptureConfig::default(),
            display_lab: DisplayLabConfig::default(),
            nice: 10,
            allowed_origins: Vec::new(),
            brand_app_id: None,
            brand_qml_module: None,
        }
    }
}

/// The display lab's switch.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct DisplayLabConfig {
    pub enabled: bool,
}

impl Default for DisplayLabConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Pictures for the shell. The shell asks `/v1/image/{root}/{file}` and never names a path: a root is a folder the owner's config
/// (or the package) names, a file is a name directly inside it.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct ImagesConfig {
    /// Off: every picture is served as the file it is, and the shell decodes it.
    pub enabled: bool,
    /// The 32-bit decoder (`jpeg/service.c`, over LG's hardware JPEG block, with libjpeg as its own fallback). Empty: `bin/lumatv-jpeg`
    /// in the data dir, where `lumatv` stages it.
    pub decoder: String,
    /// Extra arguments for the decoder: the hardware buffer's row and plane alignment (`-A 64 -B 16`) and the colour range (`-L`)
    /// as the TV slot finds them (`docs/research/hwjpeg-result.md`).
    pub decoder_args: Vec<String>,
    /// Folders by name, each an absolute path, added to the two the helper knows (`images`, the data dir's `images/`, and `art`,
    /// the package's paintings).
    pub roots: std::collections::BTreeMap<String, String>,
    /// The most pixels of one decoded picture (after the scaled decode); a larger one is served as the file.
    pub max_pixels: u32,
}

impl Default for ImagesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            decoder: String::new(),
            decoder_args: Vec::new(),
            roots: std::collections::BTreeMap::new(),
            max_pixels: 16 * 1024 * 1024,
        }
    }
}

/// Screen capture. `enabled` is the switch and the helper's own part of it; the rest is read by the compositor, which re-reads this file on
/// every capture command (a change of the switch needs no restart there), and is checked here so a typo fails at the helper's start.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct CaptureConfig {
    /// The switch. Off: `/v1/capture/...` answers 403 and the compositor refuses every capture command.
    pub enabled: bool,
    /// The compositor's control socket, which the routes forward to.
    pub control: String,
    /// What every file in `captures/` may take together; the oldest go first when a new one needs the room.
    pub max_bytes: u64,
    /// The longest one recording runs.
    pub max_record_seconds: u64,
    pub record: CaptureRecordConfig,
    /// The hardware encoder's program (`lumatv-venc`); empty: `bin/lumatv-venc` in the data dir.
    pub encoder: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct CaptureRecordConfig {
    /// The recording's height in pixels: 360, 540, 720 or 1080 (the width follows the screen's shape).
    pub height: u32,
    pub fps: u32,
    pub kbps: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            control: "/tmp/xdg/lumatv-control".to_owned(),
            max_bytes: 256 * 1024 * 1024,
            max_record_seconds: 300,
            record: CaptureRecordConfig::default(),
            encoder: String::new(),
        }
    }
}

impl Default for CaptureRecordConfig {
    fn default() -> Self {
        Self {
            height: 720,
            fps: 30,
            kbps: 4000,
        }
    }
}

/// The cameras' tuning. There is no switch: the link file (`scrypted-link`) is what turns them on.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct CamerasConfig {
    /// How long a camera's newest still is served before it counts as no picture, in milliseconds (the shell keeps its own last picture,
    /// dimmed, past this).
    pub frame_hold_ms: u32,
    /// The most a camera's picture may be, in bytes.
    pub max_picture_bytes: u32,
    /// The live path (`live/`): the full view plays the camera's stream on the video plane through the GFN feeder. Off: the full view and
    /// the grid draw pictures only.
    pub live: bool,
    /// The feeder program (`lumatv-gfn-feeder`); empty: `bin/lumatv-gfn-feeder` in the data dir, where `lumatv` stages it.
    pub feeder: String,
    /// The window type the feeder owns for a camera (`card`, `overlay`, `screensaver`): the TV check decides which shows over an app.
    pub feeder_window: String,
    /// The NDL library the feeder loads (`-L`); empty for its default.
    pub feeder_library: String,
    /// How long the feeder process is kept after the last stream, so that the next open skips its start (0: it ends with the stream).
    pub feeder_idle_seconds: u32,
    /// The window type the feeder owns for a camera pop-up: `video`, the compositor's own video window (`--vwin`), played on once a claim is granted for the
    /// card's picture; the full view's is `feeder_window`.
    pub popup_window: String,
    /// The app the feeder's decoder is opened for. LG's video path connects a video to its sink only for an app on SAM's foreground list, which the
    /// video window's own id is not: `auto` (Home's id while the window is the display's main one, its own in the sub one), `home`, `own`, or an app id.
    pub ndl_app: String,
    /// Whether a camera pop-up plays live video where the compositor gives it a window. Off: it shows stills only.
    pub popup_live: bool,
    /// `POST /v1/cameras/event` makes a pop-up as a detector would, for the TV check and the tests. Off: the route refuses.
    pub test_events: bool,
}

impl Default for CamerasConfig {
    fn default() -> Self {
        Self {
            frame_hold_ms: 10_000,
            max_picture_bytes: 4 * 1024 * 1024,
            live: true,
            feeder: String::new(),
            feeder_window: "card".to_owned(),
            feeder_library: String::new(),
            feeder_idle_seconds: 120,
            popup_window: "video".to_owned(),
            ndl_app: "auto".to_owned(),
            popup_live: true,
            test_events: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Listen {
    /// A loopback address. Anything else is refused: opening the LAN is a decision of the owner, made on the TV.
    pub host: String,
    /// 0 asks the system for a free port (tests).
    pub port: u16,
}

impl Default for Listen {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port: DEFAULT_PORT,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct LogConfig {
    /// `error`, `warn`, `info` or `debug`.
    pub level: String,
    /// The size at which the log file becomes `log-helper.1` and a new one starts.
    pub max_bytes: u64,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
            max_bytes: 256 * 1024,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct ScreenConfig {
    /// `luna` reads the power state from `luna-send`; `on` treats the screen as always on (development on a host).
    pub source: String,
    /// The poll period when the power-state subscription is not available.
    pub poll_ms: u64,
}

impl Default for ScreenConfig {
    fn default() -> Self {
        Self {
            source: "luna".to_owned(),
            poll_ms: 5_000,
        }
    }
}

/// How the receiver is read. The address and the on/off switch are settings (`base.receiver`).
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct ReceiverTuning {
    /// The period between the queries that re-confirm the receiver's state on the telnet link, between HTTP reads while that
    /// link is down, and the base of the wait before a reconnect (doubling up to `maxBackoffMs`).
    pub poll_ms: u64,
    /// The period during a burst, which follows a nudge (a volume key was pressed) so the change is seen within a fraction of a second.
    pub fast_poll_ms: u64,
    /// How long a burst lasts.
    pub burst_ms: u64,
    /// The longest wait between reads while the receiver does not answer.
    pub max_backoff_ms: u64,
    /// A reading older than this is hidden rather than shown (the plan's 6 s).
    pub hide_after_ms: u64,
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    /// The receiver's telnet port, held open for pushed changes while the screen is on. The HTTP reads are then only the
    /// fallback while it is down. 0 turns the telnet link off.
    pub telnet_port: u16,
    /// The shortest time between two volume commands to the receiver (the display's level is sent coalesced: 100 ms is 10 a second).
    pub volume_gap_ms: u64,
    /// How long the level has to be quiet before the receiver is read once to see that it took it, and corrected if it did not.
    pub volume_settle_ms: u64,
    /// The most corrections after one level.
    pub volume_retries: u32,
}

impl Default for ReceiverTuning {
    fn default() -> Self {
        Self {
            poll_ms: 2_000,
            fast_poll_ms: 250,
            burst_ms: 3_000,
            max_backoff_ms: 30_000,
            hide_after_ms: 6_000,
            connect_timeout_ms: 1_500,
            request_timeout_ms: 3_000,
            telnet_port: lumatv_denon::telnet::DEFAULT_TELNET_PORT,
            volume_gap_ms: 100,
            volume_settle_ms: 400,
            volume_retries: 3,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct FetchConfig {
    /// `auto` uses LumaShow's `fetch` (`openssl s_client`) when the set's `openssl` is version 3 or later and `curl` otherwise;
    /// `openssl` and `curl` force one.
    pub backend: String,
    /// The `openssl` program LumaShow's `fetch` runs as `s_client`.
    pub openssl: String,
    /// The `curl` program, for the fallback.
    pub curl: String,
    /// The CA bundle; unset means the first of the usual places that exists.
    pub ca_bundle: Option<String>,
    /// The user the TLS child runs as when the helper is root, so the program that parses certificates holds no privilege.
    /// `null` runs it as the helper does.
    #[serde(default = "default_fetch_user")]
    pub user: Option<String>,
}

fn default_fetch_user() -> Option<String> {
    Some("nobody".to_owned())
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            backend: "auto".to_owned(),
            openssl: fetch::IMAGE_OPENSSL.to_owned(),
            curl: "curl".to_owned(),
            ca_bundle: None,
            user: default_fetch_user(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct LunaConfig {
    /// The `luna-send` program (a path, or a name looked up in `PATH`).
    pub send: String,
}

impl Default for LunaConfig {
    fn default() -> Self {
        Self {
            send: "luna-send".to_owned(),
        }
    }
}

/// The output rate after 25 or 50 fps video (`framerate.rs`).
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct FrameRateConfig {
    /// Puts the panel back to 120 Hz when Home is in front with no video window after a 25 or 50 fps video left it at 100 Hz.
    /// Off: the rate stays as LG's kernel left it until the next video.
    pub return_on_home: bool,
}

impl Default for FrameRateConfig {
    fn default() -> Self {
        Self {
            return_on_home: true,
        }
    }
}

/// How the AI-access module runs. Nothing here switches it on: that is a setting only a physical remote press can change.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct AiConfig {
    /// The port of the LAN listener while it is on; 0 asks the system for one (tests).
    pub port: u16,
    /// The LAN address to bind. Unset: the private address of the default route's interface. Never a public address.
    pub bind: Option<String>,
    /// Source networks that may connect (CIDR). Private ranges by default; a peer outside them is dropped unanswered.
    pub allowed_nets: Vec<String>,
    /// Input devices that are physical remotes. A name ending in `*` is a prefix. Only keys from these answer a prompt.
    pub physical_devices: Vec<String>,
    /// Input devices that carry network keys (named for the log; any device not listed as physical is ignored anyway).
    pub network_devices: Vec<String>,
    /// Evdev key codes that approve a prompt (OK) and that deny it (Back).
    pub approve_codes: Vec<u16>,
    pub deny_codes: Vec<u16>,
    /// How long a prompt waits for the remote.
    pub prompt_seconds: u64,
    /// Where the input devices are: their nodes and their names (changed in tests).
    pub input_dir: String,
    pub sys_input_dir: String,
    /// The device that carries network keys, which the media keys are pressed through.
    pub inject_device: String,
    pub media_keys: MediaKeys,
}

/// The evdev codes of the playback keys an assistant can press (the standard ones; not yet measured on the set).
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct MediaKeys {
    pub play: u16,
    pub pause: u16,
    pub stop: u16,
    pub rewind: u16,
    pub fast_forward: u16,
}

impl Default for MediaKeys {
    fn default() -> Self {
        Self {
            play: 207,
            pause: 119,
            stop: 128,
            rewind: 168,
            fast_forward: 208,
        }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            port: 18732,
            bind: None,
            allowed_nets: ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16"]
                .map(str::to_owned)
                .to_vec(),
            physical_devices: ["LGE RCU", "LGE M-RCU*"].map(str::to_owned).to_vec(),
            network_devices: ["Smart Remote RCU Input", "LGE Network Input"]
                .map(str::to_owned)
                .to_vec(),
            approve_codes: vec![28, 352],
            deny_codes: vec![412, 158, 174],
            prompt_seconds: 30,
            input_dir: "/dev/input".to_owned(),
            sys_input_dir: "/sys/class/input".to_owned(),
            inject_device: "Smart Remote RCU Input".to_owned(),
            media_keys: MediaKeys::default(),
        }
    }
}

/// How the plugin broker runs. Which plugins are on is not here: it is `plugins/enabled.json` in the data dir, which the helper
/// never writes and no API of it can change.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct PluginsConfig {
    /// The broker's unix socket. Plugins find it at LumaShow's path and nowhere else, so this changes only in tests.
    pub socket: String,
    /// How often `enabled.json` is read again while the helper runs (a plugin switched on later is noticed within this).
    pub rescan_seconds: u64,
    /// How long a connection has to say `hello`, and how long a question to a plugin (an action) waits for its answer.
    pub handshake_seconds: u64,
    pub answer_seconds: u64,
    /// The user id of plugin 0: a plugin runs as this id plus its number (0 to 63). Only a test changes it, to the id it runs as.
    pub first_uid: u32,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            socket: plugin_api::BROKER_SOCKET.to_owned(),
            rescan_seconds: 30,
            handshake_seconds: plugin_api::limits::HANDSHAKE_SECS,
            answer_seconds: plugin_api::limits::ANSWER_SECS,
            first_uid: plugin_api::PLUGIN_UID_BASE,
        }
    }
}

/// Where the ambient lights are. Which of them to drive, how bright and when are settings (`base.ambient`, the Settings page); an
/// address on the network is the owner's to give, here, so the helper never finds a device by itself and never sends the picture's
/// colours to one that is not named.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct AmbientConfig {
    /// The WLED strip (DDP over UDP). Unset: no strip, and ambient light has nothing to drive.
    pub wled: Option<WledConfig>,
    /// Where the 32-bit capture helper hands the picture's areas to this one (a unix socket). Unset: `ambient.sock` in the data dir.
    pub capture_socket: Option<String>,
    /// Where the sound for the sound modes of ambient light comes from.
    pub sound: SoundConfig,
    /// The Zigbee2MQTT output's protection of the Zigbee network.
    pub z2m: Z2mConfig,
}

/// How the Zigbee2MQTT output protects the Zigbee network (`ambient_z2m.rs`). The defaults are what a real network needs (one
/// stream aborted an OTA update of a battery sensor); only a test shortens them.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Z2mConfig {
    /// How long after the last update of any device finished before frames are sent again.
    pub ota_settle_ms: u64,
    /// An update not heard of for this long is taken as over, so a missed end cannot block the lights for good.
    pub ota_stale_ms: u64,
    /// How long the stream stays off after repeated late answers from the coordinator.
    pub busy_pause_ms: u64,
    /// Late answers closer together than this are one event, and an event halves the frame rate once.
    pub busy_cut_gap_ms: u64,
    /// Without a late answer for this long, the frame rate goes up by half again, to the link's rate.
    pub recover_ms: u64,
}

impl Default for Z2mConfig {
    fn default() -> Self {
        Self {
            ota_settle_ms: 30_000,
            ota_stale_ms: 300_000,
            busy_pause_ms: 60_000,
            busy_cut_gap_ms: 2_000,
            recover_ms: 20_000,
        }
    }
}

/// The TV's own sound mix for ambient light's sound modes (`ambient_audio_capture.rs`). Whether the lights use it is the
/// `sound` setting (`base.ambient`); where it is read from is the owner's to choose, here.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct SoundConfig {
    /// The program that writes the mix to its standard output as raw 48 kHz, 16-bit, two-channel, little-endian PCM, with its
    /// arguments. It runs only while a sound mode is on and the lights are lit. The default is `arecord` on card 1 `LGSIC`,
    /// device 2 `MixerCapture`, which carries the TV's sound before its volume and before the receiver's mute; no microphone is
    /// involved. Empty: no sound modes.
    pub capture: Vec<String>,
}

/// The mix as the TV's own sound card gives it.
pub const DEFAULT_SOUND_CAPTURE: [&str; 12] = [
    "arecord", "-q", "-D", "hw:1,2", "-t", "raw", "-f", "S16_LE", "-r", "48000", "-c", "2",
];

impl Default for SoundConfig {
    fn default() -> Self {
        Self {
            capture: DEFAULT_SOUND_CAPTURE
                .iter()
                .map(|a| (*a).to_owned())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WledConfig {
    /// An IPv4 address on the home network (loopback, private or link-local): a picture's colours never leave it.
    pub host: String,
    #[serde(default = "default_ddp_port")]
    pub port: u16,
}

fn default_ddp_port() -> u16 {
    4048
}

/// How voice search listens. Nothing here switches it on (that is the `base.voice` setting) or says where the audio goes (the saved
/// link, written by `lumatv-helper voice-link`); it is where the audio comes from, which is the owner's to choose.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct VoiceConfig {
    /// The program that writes the microphone to its standard output as raw 16 kHz, 16-bit, mono, little-endian PCM, with its
    /// arguments. It runs only while the mic key is held. The default is the TV's own far-field microphone through ALSA (card 1
    /// `LGSIC`, device 0 "WoV PDM Mic", which offers S24_LE only; `plughw` converts); LG's voice services stay stopped. Empty: no
    /// microphone, and voice search stays unavailable.
    pub capture: Vec<String>,
    /// The longest one request listens, whatever the key does: a missed key release must not leave the microphone open.
    pub max_seconds: u64,
    /// How long the microphone stays open after the key is released, so the last word is not cut.
    pub tail_ms: u64,
}

/// The TV's built-in microphone, read by `arecord` as 16 kHz mono S16_LE (Nitin, 2026-10-25).
pub const DEFAULT_CAPTURE: [&str; 12] = [
    "arecord",
    "-q",
    "-D",
    "plughw:1,0",
    "-t",
    "raw",
    "-f",
    "S16_LE",
    "-r",
    "16000",
    "-c",
    "1",
];

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            capture: DEFAULT_CAPTURE.iter().map(|a| (*a).to_owned()).collect(),
            max_seconds: 20,
            tail_ms: 250,
        }
    }
}

/// What the device is (`device.rs`): the TV unless `helper.json` says otherwise. The screen and the remote are facts of the
/// device; a game controller is looked for in the kernel's input list each time it is asked.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct DeviceConfig {
    /// A Lucent device profile: `tv`, `smart`, `phone`, `tablet` or `desktop`.
    pub screen_class: String,
    /// The logical pixels the UI is drawn in (the TV draws 1920 by 1080 and LG's scaler fills the panel).
    pub width: u32,
    pub height: u32,
    pub touch: bool,
    /// The D-pad remote.
    pub remote: bool,
    /// The Magic Remote's pointer. Its on/off switch is LG's (`general.remotePointer`); this says the TV has one.
    pub pointer: bool,
    /// Whether a connected game controller adds `controller` to the input methods.
    pub detect_controllers: bool,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            screen_class: "tv".to_owned(),
            width: 1920,
            height: 1080,
            touch: false,
            remote: true,
            pointer: true,
            detect_controllers: true,
        }
    }
}

/// The CA bundles a Linux image keeps, in the order they are tried when the config names none.
pub const CA_BUNDLE_CANDIDATES: [&str; 5] = [
    fetch::IMAGE_CA_BUNDLE,
    "/etc/ssl/certs/ca-bundle.crt",
    "/etc/pki/tls/certs/ca-bundle.crt",
    "/etc/ssl/cert.pem",
    "/etc/ssl/ca-bundle.pem",
];

/// The keys of `cameras` the Home Assistant camera path had and the Scrypted cameras do not.
const OLD_CAMERAS_KEYS: [&str; 6] = [
    "enabled",
    "port",
    "relay",
    "popupKinds",
    "popupSeconds",
    "cooldownSeconds",
];

/// The document without the old `cameras` keys, when it had any (and says which in the log); `None` when it had none or is not JSON.
fn without_old_cameras_keys(text: &str) -> Option<serde_json::Value> {
    let mut value: serde_json::Value = serde_json::from_str(text).ok()?;
    let cameras = value.get_mut("cameras")?.as_object_mut()?;
    let dropped: Vec<&str> = OLD_CAMERAS_KEYS
        .into_iter()
        .filter(|key| cameras.remove(*key).is_some())
        .collect();
    if dropped.is_empty() {
        return None;
    }
    crate::log_warn!(
        "helper.json: cameras.{} belonged to the Home Assistant camera path and is ignored (the cameras come through Scrypted now)",
        dropped.join(", cameras.")
    );
    Some(value)
}

impl Config {
    /// Parses a config document and checks what serde cannot. A file written for the Home Assistant camera path (the TV's own was
    /// `{"cameras": {"enabled": true}}`) is read all the same: the keys that path had are dropped, once, and are said in the log; a refused
    /// config would be a helper that does not start after the update.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let config: Self = match serde_json::from_str(text) {
            Ok(config) => config,
            Err(first) => {
                let Some(value) = without_old_cameras_keys(text) else {
                    return Err(ConfigError(format!("helper.json: {first}")));
                };
                serde_json::from_value(value)
                    .map_err(|e| ConfigError(format!("helper.json: {e}")))?
            }
        };
        config.check()?;
        Ok(config)
    }

    /// Reads `path`; a missing file is the defaults, any other problem is an error.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                config.check()?;
                Ok(config)
            }
            Err(e) => Err(ConfigError(format!("{}: {e}", path.display()))),
        }
    }

    fn check(&self) -> Result<(), ConfigError> {
        let err = |m: &str| Err(ConfigError(format!("helper.json: {m}")));
        match self.listen.host.parse::<IpAddr>() {
            Ok(ip) if ip.is_loopback() => {}
            _ => return err("listen.host must be a loopback address (127.0.0.1 or ::1)"),
        }
        if crate::log::Level::parse(&self.log.level).is_none() {
            return err("log.level must be error, warn, info or debug");
        }
        if !matches!(self.screen.source.as_str(), "luna" | "on") {
            return err("screen.source must be luna or on");
        }
        for (name, value) in [
            ("screen.pollMs", self.screen.poll_ms),
            ("receiver.pollMs", self.receiver.poll_ms),
            ("receiver.fastPollMs", self.receiver.fast_poll_ms),
            ("receiver.maxBackoffMs", self.receiver.max_backoff_ms),
            ("receiver.hideAfterMs", self.receiver.hide_after_ms),
            (
                "receiver.connectTimeoutMs",
                self.receiver.connect_timeout_ms,
            ),
            (
                "receiver.requestTimeoutMs",
                self.receiver.request_timeout_ms,
            ),
            ("receiver.volumeGapMs", self.receiver.volume_gap_ms),
            ("receiver.volumeSettleMs", self.receiver.volume_settle_ms),
        ] {
            if value == 0 {
                return err(&format!("{name} must be above 0"));
            }
        }
        if let Some(bind) = &self.ai.bind {
            match bind.parse::<std::net::Ipv4Addr>() {
                Ok(ip) if crate::sys::is_lan_address(ip) => {}
                _ => return err("ai.bind must be a private (or loopback) IPv4 address"),
            }
        }
        if self.ai.allowed_nets.is_empty()
            || self
                .ai
                .allowed_nets
                .iter()
                .any(|n| crate::http::Cidr::parse(n).is_none())
        {
            return err(
                "ai.allowedNets must be a non-empty list of CIDR networks such as 192.168.0.0/16",
            );
        }
        if self.ai.physical_devices.is_empty()
            || self.ai.physical_devices.iter().any(String::is_empty)
        {
            return err("ai.physicalDevices must name at least one device");
        }
        if self.ai.approve_codes.is_empty() || self.ai.deny_codes.is_empty() {
            return err("ai.approveCodes and ai.denyCodes must not be empty");
        }
        if self
            .ai
            .approve_codes
            .iter()
            .any(|c| self.ai.deny_codes.contains(c))
        {
            return err("ai.approveCodes and ai.denyCodes must not share a code");
        }
        if !(5..=120).contains(&self.ai.prompt_seconds) {
            return err("ai.promptSeconds must be between 5 and 120");
        }
        if self.ai.input_dir.is_empty()
            || self.ai.sys_input_dir.is_empty()
            || self.ai.inject_device.is_empty()
        {
            return err("ai.inputDir, ai.sysInputDir and ai.injectDevice must not be empty");
        }
        if !self.plugins.socket.starts_with('/') {
            return err("plugins.socket must be an absolute path");
        }
        if !(1..=3600).contains(&self.plugins.rescan_seconds)
            || !(1..=60).contains(&self.plugins.handshake_seconds)
            || !(1..=60).contains(&self.plugins.answer_seconds)
        {
            return err(
                "plugins.rescanSeconds is 1 to 3600, plugins.handshakeSeconds and plugins.answerSeconds are 1 to 60",
            );
        }
        if self.plugins.first_uid > u32::MAX - plugin_api::MAX_PLUGINS {
            return err("plugins.firstUid leaves no room for 64 plugin user ids");
        }
        if !matches!(
            self.device.screen_class.as_str(),
            "tv" | "smart" | "phone" | "tablet" | "desktop"
        ) {
            return err("device.screenClass must be tv, smart, phone, tablet or desktop");
        }
        if !(1..=16_384).contains(&self.device.width) || !(1..=16_384).contains(&self.device.height)
        {
            return err("device.width and device.height are 1 to 16384");
        }
        if let Some(wled) = &self.ambient.wled {
            match wled.host.parse::<std::net::Ipv4Addr>() {
                Ok(ip) if crate::sys::is_lan_address(ip) && wled.port != 0 => {}
                _ => {
                    return err(
                        "ambient.wled.host must be a private (or loopback) IPv4 address and ambient.wled.port above 0",
                    );
                }
            }
        }
        if self
            .ambient
            .capture_socket
            .as_deref()
            .is_some_and(|p| !p.starts_with('/'))
        {
            return err("ambient.captureSocket must be an absolute path");
        }
        if self
            .ambient
            .sound
            .capture
            .first()
            .is_some_and(String::is_empty)
            || self.ambient.sound.capture.iter().any(|a| a.contains('\0'))
        {
            return err(
                "ambient.sound.capture is a program and its arguments, none empty or holding a NUL",
            );
        }
        if self.voice.capture.first().is_some_and(String::is_empty)
            || self.voice.capture.iter().any(|a| a.contains('\0'))
        {
            return err(
                "voice.capture is a program and its arguments, none empty or holding a NUL",
            );
        }
        if !(1..=120).contains(&self.voice.max_seconds) || self.voice.tail_ms > 5_000 {
            return err("voice.maxSeconds is 1 to 120 and voice.tailMs at most 5000");
        }
        let images = &self.images;
        if !images.decoder.is_empty() && !images.decoder.starts_with('/') {
            return err("images.decoder must be an absolute path");
        }
        if images.decoder_args.len() > 12
            || images
                .decoder_args
                .iter()
                .any(|a| a.is_empty() || a.contains('\0'))
        {
            return err("images.decoderArgs is at most 12 arguments, none empty or holding a NUL");
        }
        if images.roots.len() > 16
            || images.roots.iter().any(|(name, dir)| {
                !crate::images::root_name_ok(name) || !dir.starts_with('/') || dir.contains('\0')
            })
        {
            return err(
                "images.roots is at most 16 folders: a lowercase name (letters, digits, dashes) and an absolute path each",
            );
        }
        if !(1..=64 * 1024 * 1024).contains(&images.max_pixels) {
            return err("images.maxPixels is 1 to 67108864");
        }
        let capture = &self.capture;
        if !capture.control.starts_with('/')
            || capture.control.contains('\0')
            || (!capture.encoder.is_empty() && !capture.encoder.starts_with('/'))
        {
            return err("capture.control and capture.encoder must be absolute paths");
        }
        if capture.max_bytes < 16 * 1024 * 1024
            || !(5..=1800).contains(&capture.max_record_seconds)
            || ![360, 540, 720, 1080].contains(&capture.record.height)
            || !(5..=30).contains(&capture.record.fps)
            || !(250..=20_000).contains(&capture.record.kbps)
        {
            return err(
                "capture.maxBytes is at least 16777216, maxRecordSeconds 5 to 1800, record.height 360, 540, 720 or 1080, record.fps 5 to 30, record.kbps 250 to 20000",
            );
        }
        if !(0..=19).contains(&self.nice) {
            return err("nice must be between 0 and 19");
        }
        if !matches!(self.fetch.backend.as_str(), "auto" | "openssl" | "curl") {
            return err("fetch.backend must be auto, openssl or curl");
        }
        if self.fetch.openssl.is_empty() || self.fetch.curl.is_empty() || self.luna.send.is_empty()
        {
            return err("fetch.openssl, fetch.curl and luna.send must not be empty");
        }
        Ok(())
    }
}

/// Reads the data dir from a `brand.json`: the same rule as `tools/brand.sh` (a directory directly under `/var/lib`, lowercase
/// letters, digits and dashes), because the helper must never be pointed outside the places the mod writes.
pub fn data_dir_from_brand(brand_json: &str) -> Result<PathBuf, ConfigError> {
    let value: serde_json::Value =
        serde_json::from_str(brand_json).map_err(|e| ConfigError(format!("brand.json: {e}")))?;
    let dir = value
        .get("dataDir")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ConfigError("brand.json: dataDir is missing".to_owned()))?;
    let name = dir
        .strip_prefix("/var/lib/")
        .filter(|n| {
            !n.is_empty()
                && n.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        .ok_or_else(|| {
            ConfigError(
                "brand.json: dataDir must be /var/lib/<lowercase letters, digits, dashes>"
                    .to_owned(),
            )
        })?;
    Ok(PathBuf::from(format!("/var/lib/{name}")))
}

/// The app id from a `brand.json` (LumaTV's own app, which an assistant must never be able to open), by the tool's own rule:
/// lowercase letters, digits, dots and dashes. `None` when it is missing or not of that shape.
pub fn app_id_from_brand(brand_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(brand_json).ok()?;
    let id = value.get("appId")?.as_str()?;
    (!id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'))
    .then(|| id.to_owned())
}

/// The QML module from a `brand.json` (`qmlModule`), by the tool's own rule: lowercase letters and digits. `None` when it is missing
/// or not of that shape.
pub fn qml_module_from_brand(brand_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(brand_json).ok()?;
    let name = value.get("qmlModule")?.as_str()?;
    (!name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
    .then(|| name.to_owned())
}

/// The first CA bundle that exists, as the config or the usual places give it.
pub fn find_ca_bundle(config: &FetchConfig) -> Option<PathBuf> {
    if let Some(path) = &config.ca_bundle {
        return Some(PathBuf::from(path));
    }
    CA_BUNDLE_CANDIDATES
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_listens_to_the_tvs_own_microphone_unless_told_otherwise() {
        let config = Config::parse("{}").unwrap();
        assert_eq!(
            config.voice.capture.join(" "),
            "arecord -q -D plughw:1,0 -t raw -f S16_LE -r 16000 -c 1"
        );
        // Emptying it is how the owner takes the microphone away.
        assert!(
            Config::parse(r#"{"voice":{"capture":[]}}"#)
                .unwrap()
                .voice
                .capture
                .is_empty()
        );
        for bad in [
            r#"{"voice":{"capture":[""]}}"#,
            r#"{"voice":{"maxSeconds":0}}"#,
            r#"{"voice":{"tailMs":9000}}"#,
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_plugin_broker_defaults_to_lumashows_socket_and_rejects_nonsense() {
        let config = Config::parse("{}").unwrap();
        assert_eq!(config.plugins.socket, "/run/lumashow-plugins/broker.sock");
        assert_eq!(config.plugins.first_uid, 20_000);
        for bad in [
            r#"{"plugins":{"socket":"relative/path"}}"#,
            r#"{"plugins":{"rescanSeconds":0}}"#,
            r#"{"plugins":{"handshakeSeconds":61}}"#,
            r#"{"plugins":{"firstUid":4294967290}}"#,
            r#"{"plugins":{"enabled":true}}"#,
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_device_is_the_tv_unless_the_file_says_otherwise_and_nonsense_is_refused() {
        let tv = Config::parse("{}").unwrap().device;
        assert_eq!(
            (tv.screen_class.as_str(), tv.width, tv.height),
            ("tv", 1920, 1080)
        );
        assert!(tv.remote && tv.pointer && tv.detect_controllers && !tv.touch);
        let smart = Config::parse(
            r#"{"device":{"screenClass":"smart","width":960,"height":480,"touch":true,"remote":false}}"#,
        )
        .unwrap()
        .device;
        assert!(smart.touch && !smart.remote);
        for bad in [
            r#"{"device":{"screenClass":"fridge"}}"#,
            r#"{"device":{"width":0}}"#,
            r#"{"device":{"height":100000}}"#,
            r#"{"device":{"inputs":["remote"]}}"#,
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn no_ambient_device_is_named_until_the_owner_names_one_and_only_on_the_home_network() {
        let none = Config::parse("{}").unwrap().ambient;
        assert!(none.wled.is_none() && none.capture_socket.is_none());
        let named = Config::parse(r#"{"ambient":{"wled":{"host":"192.168.1.50"}}}"#)
            .unwrap()
            .ambient;
        let wled = named.wled.unwrap();
        assert_eq!((wled.host.as_str(), wled.port), ("192.168.1.50", 4048));
        assert!(Config::parse(r#"{"ambient":{"wled":{"host":"127.0.0.1","port":4049}}}"#).is_ok());
        for bad in [
            r#"{"ambient":{"wled":{"host":"8.8.8.8"}}}"#,
            r#"{"ambient":{"wled":{"host":"strip.example.com"}}}"#,
            r#"{"ambient":{"wled":{"host":"192.168.1.50","port":0}}}"#,
            r#"{"ambient":{"wled":{}}}"#,
            r#"{"ambient":{"captureSocket":"relative.sock"}}"#,
            r#"{"ambient":{"hue":{"host":"192.168.1.60"}}}"#,
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_sound_for_ambient_light_is_the_tvs_mix_unless_the_owner_names_another_or_none() {
        let config = Config::parse("{}").unwrap();
        assert_eq!(
            config.ambient.sound.capture.join(" "),
            "arecord -q -D hw:1,2 -t raw -f S16_LE -r 48000 -c 2"
        );
        let other =
            Config::parse(r#"{"ambient":{"sound":{"capture":["cat","/tmp/x.pcm"]}}}"#).unwrap();
        assert_eq!(other.ambient.sound.capture, ["cat", "/tmp/x.pcm"]);
        // Emptying it is how the owner takes the sound modes away.
        let none = Config::parse(r#"{"ambient":{"sound":{"capture":[]}}}"#).unwrap();
        assert!(none.ambient.sound.capture.is_empty());
        for bad in [
            r#"{"ambient":{"sound":{"capture":[""]}}}"#,
            r#"{"ambient":{"sound":{"mic":"x"}}}"#,
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn no_file_means_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load(&dir.path().join("helper.json")).unwrap();
        assert_eq!(config.listen.port, DEFAULT_PORT);
        assert_eq!(config.listen.host, "127.0.0.1");
        assert_eq!(config.receiver.hide_after_ms, 6_000);
        assert_eq!(config.fetch.user.as_deref(), Some("nobody"));
        assert!(config.allowed_origins.is_empty());
        assert_eq!(config.nice, 10);
    }

    #[test]
    fn a_partial_file_overrides_only_what_it_names() {
        let config =
            Config::parse(r#"{"listen":{"port":19000},"receiver":{"pollMs":5000}}"#).unwrap();
        assert_eq!(config.listen.port, 19000);
        assert_eq!(config.listen.host, "127.0.0.1");
        assert_eq!(config.receiver.poll_ms, 5000);
        assert_eq!(config.receiver.fast_poll_ms, 250);
    }

    #[test]
    fn a_typo_is_an_error_not_a_default() {
        let e = Config::parse(r#"{"listn":{"port":1}}"#).unwrap_err();
        assert!(e.0.contains("listn"), "{e}");
        let e = Config::parse(r#"{"receiver":{"pollms":1}}"#).unwrap_err();
        assert!(e.0.contains("pollms"), "{e}");
    }

    #[test]
    fn the_listener_cannot_be_moved_off_loopback() {
        for host in ["0.0.0.0", "192.168.1.5", "::", "tv.local", ""] {
            let doc = format!(r#"{{"listen":{{"host":"{host}"}}}}"#);
            let e = Config::parse(&doc).unwrap_err();
            assert!(e.0.contains("loopback"), "{host}: {e}");
        }
        assert!(Config::parse(r#"{"listen":{"host":"::1"}}"#).is_ok());
        assert!(Config::parse(r#"{"listen":{"host":"127.0.0.2"}}"#).is_ok());
    }

    #[test]
    fn bad_values_are_named() {
        assert!(
            Config::parse(r#"{"log":{"level":"loud"}}"#)
                .unwrap_err()
                .0
                .contains("log.level")
        );
        assert!(
            Config::parse(r#"{"screen":{"source":"magic"}}"#)
                .unwrap_err()
                .0
                .contains("screen.source")
        );
        assert!(
            Config::parse(r#"{"receiver":{"pollMs":0}}"#)
                .unwrap_err()
                .0
                .contains("receiver.pollMs")
        );
        assert!(
            Config::parse(r#"{"nice":20}"#)
                .unwrap_err()
                .0
                .contains("nice")
        );
        assert!(
            Config::parse(r#"{"nice":-1}"#)
                .unwrap_err()
                .0
                .contains("nice")
        );
    }

    #[test]
    fn the_fetch_user_can_be_switched_off() {
        let config = Config::parse(r#"{"fetch":{"user":null}}"#).unwrap();
        assert_eq!(config.fetch.user, None);
        let config = Config::parse(r#"{"fetch":{"user":"daemon"}}"#).unwrap();
        assert_eq!(config.fetch.user.as_deref(), Some("daemon"));
    }

    #[test]
    fn the_data_dir_comes_from_brand_json_by_the_tools_rule() {
        let brand = r#"{"displayName":"LumaTV","dataDir":"/var/lib/lumatv"}"#;
        assert_eq!(
            data_dir_from_brand(brand).unwrap(),
            PathBuf::from("/var/lib/lumatv")
        );
        for bad in [
            r#"{"dataDir":"/var/lib/"}"#,
            r#"{"dataDir":"/var/lib/Luma"}"#,
            r#"{"dataDir":"/var/lib/a/b"}"#,
            r#"{"dataDir":"/tmp/x"}"#,
            r#"{"dataDir":"/var/lib/../etc"}"#,
            r#"{"name":"x"}"#,
            "not json",
        ] {
            assert!(data_dir_from_brand(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_ai_section_has_safe_defaults_and_refuses_unsafe_ones() {
        let config = Config::parse("{}").unwrap();
        assert_eq!(config.ai.port, 18732);
        assert_eq!(config.ai.bind, None);
        assert_eq!(config.ai.prompt_seconds, 30);
        assert_eq!(config.ai.physical_devices, ["LGE RCU", "LGE M-RCU*"]);
        assert!(
            config
                .ai
                .network_devices
                .contains(&"Smart Remote RCU Input".to_owned())
        );
        assert_eq!(config.ai.approve_codes, [28, 352]);
        assert!(config.ai.deny_codes.contains(&412));
        for bad in [
            r#"{"ai":{"bind":"8.8.8.8"}}"#,
            r#"{"ai":{"bind":"0.0.0.0"}}"#,
            r#"{"ai":{"bind":"not an address"}}"#,
            r#"{"ai":{"allowedNets":[]}}"#,
            r#"{"ai":{"allowedNets":["192.168.1.0/33"]}}"#,
            r#"{"ai":{"allowedNets":["everyone"]}}"#,
            r#"{"ai":{"physicalDevices":[]}}"#,
            r#"{"ai":{"approveCodes":[]}}"#,
            r#"{"ai":{"approveCodes":[28],"denyCodes":[28]}}"#,
            r#"{"ai":{"promptSeconds":1}}"#,
            r#"{"ai":{"promptSeconds":500}}"#,
            r#"{"ai":{"enabled":true}}"#,
            r#"{"ai":{"listen":"0.0.0.0"}}"#,
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
        for good in [
            r#"{"ai":{"bind":"192.168.1.75","port":0}}"#,
            r#"{"ai":{"bind":"127.0.0.1"}}"#,
            r#"{"ai":{"allowedNets":["192.168.1.0/24"],"promptSeconds":60}}"#,
        ] {
            assert!(Config::parse(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn nothing_in_the_config_can_switch_the_ai_listener_on() {
        // There is no key for it: the switch is a setting only a physical remote press can change.
        for key in [
            "enabled",
            "listener",
            "on",
            "start",
            "autostart",
            "token",
            "tokens",
            "pair",
            "pairing",
            "unlock",
        ] {
            let doc = format!(r#"{{"ai":{{"{key}":true}}}}"#);
            assert!(Config::parse(&doc).is_err(), "{key}");
        }
    }

    #[test]
    fn the_brand_app_id_is_read_by_the_tools_rule() {
        assert_eq!(
            app_id_from_brand(r#"{"appId":"org.nphil.lumatv"}"#).as_deref(),
            Some("org.nphil.lumatv")
        );
        for bad in [
            r#"{"appId":""}"#,
            r#"{"appId":"Org.X"}"#,
            r#"{"appId":"a b"}"#,
            r#"{"appId":5}"#,
            r#"{}"#,
            "nope",
        ] {
            assert_eq!(app_id_from_brand(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_helper_json_of_the_home_assistant_camera_path_is_read_and_its_old_keys_are_ignored() {
        // the file the TV has today
        let config = Config::parse(r#"{"cameras":{"enabled":true}}"#).unwrap();
        assert!(config.cameras.live);
        assert_eq!(
            config.cameras.frame_hold_ms,
            Config::default().cameras.frame_hold_ms
        );
        // every old key, beside one that is still there
        let config = Config::parse(
            r#"{"cameras":{"enabled":true,"port":8123,"relay":{"x":1},"popupKinds":["person"],"popupSeconds":12,"cooldownSeconds":30,"frameHoldMs":5000}}"#,
        )
        .unwrap();
        assert_eq!(config.cameras.frame_hold_ms, 5000);
        // what never was a key is still refused, with or without the old ones
        assert!(Config::parse(r#"{"cameras":{"bogus":1}}"#).is_err());
        assert!(Config::parse(r#"{"cameras":{"enabled":true,"bogus":1}}"#).is_err());
        assert!(Config::parse(r#"{"enabled":true}"#).is_err());
    }

    #[test]
    fn paths_are_under_the_data_dir() {
        let p = Paths::new("/var/lib/lumatv");
        assert_eq!(p.log_file(), PathBuf::from("/var/lib/lumatv/log-helper"));
        assert_eq!(
            p.config_file(),
            PathBuf::from("/var/lib/lumatv/helper.json")
        );
        assert_eq!(p.lock_file(), PathBuf::from("/var/lib/lumatv/helper.lock"));
        assert_eq!(p.cache_dir(), PathBuf::from("/var/lib/lumatv/cache"));
        assert_eq!(p.settings_dir(), PathBuf::from("/var/lib/lumatv/settings"));
    }
}
