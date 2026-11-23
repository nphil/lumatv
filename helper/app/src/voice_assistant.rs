//! Voice assistant: the TV's microphone as a Home Assistant voice satellite, as a plugin of the helper's own that asks first
//! (`plugins/local.rs`'s `OptIn`).
//!
//! **What runs.** While the plugin is on and the TV is paired with Home Assistant, the helper runs the sound service of `helper/voice`
//! (`lumavoice`) in a thread of its own: it opens the TV's microphone and listens for the wake word on the TV itself, and nothing leaves
//! the TV or is kept before it. `voice-host` speaks ESPHome's voice-assistant protocol to Home Assistant, which does the speech to text,
//! the intent and the answer. Off, or not paired, the microphone is not opened at all.
//!
//! **Pairing** is Home Assistant's own add-device flow: the page opens a window of five minutes in which the TV announces itself
//! (`_esphomelib._tcp`) and lets Home Assistant set a key, kept in `voice-assistant/key` (mode 0600). With neither the window nor a key
//! nothing listens (`pair.rs`).
//!
//! **Quiet hours** (01:00 to 07:00 unless the page says otherwise) answer in text only: the reply is shown and not played. Nothing else is
//! touched: the TV's own audio, the receiver and LG's audio processor stay as they were.
//!
//! **The screen** is told by the `voice.assistant` topic (`turn.rs`): the microphone's state, and the turn's words while it happens. Another
//! Home Assistant assistant in the TV's room is shown the same way (`dot.rs`).
//!
//! **The audio processor guard.** LG's audio processor counts its resets (`aud_dsp_reset_count`). If the count rises while the microphone
//! is open, the plugin closes the microphone and says so until it is switched off and on again.

mod dot;
mod pair;
mod turn;

use crate::API_VERSION;
use crate::ble::{MdnsAnnouncer, interface_mac};
use crate::ctx::Ctx;
use crate::http::{Request, Response, error_response};
use crate::hub::Hub;
use crate::plugins::firstparty::{clip, local_time};
use crate::plugins::local::{Handler, Local, OptIn, Spec};
use crate::plugins::session::{MAX_FACT_LINE, Shared};
use crate::timeutil::now_ms;
use dot::{Follower, Params};
use lumavoice::input::tv_mic_format;
use lumavoice::models::{self, Sensitivity};
use lumavoice::{Alsa, Aplay, Format, Handle, Sample, Status};
use pair::{Announcement, KeyFile, Window};
use parking_lot::{Mutex, RwLock};
use plugin_api::wire::ToPlugin;
use serde_json::{Map, Value as Json, json};
use settings_schema::{Value as Setting, Values};
use std::io;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use turn::{Mic, Other, Overlay};
use voice_host::{Access, Arbitration, Env, Host, Mode, Services, Settings, WakeWord};
use voice_satellite::{Config as Satellite, Effect, Identity, WakeWordInfo};
use voice_service::{Chime, Shown};

/// The plugin's id, which is also its settings owner.
pub const ID: &str = "voice-assistant";
/// What the screen is told: the microphone's state and the turn that is going (`turn.rs`).
pub const TOPIC: &str = "voice.assistant";
pub const ROUTES: &[(&str, &str, &str)] = &[(
    "POST",
    "/v1/voice-assistant/selftest",
    "plays the recorded wake phrase kept as voice-assistant/selftest.wav to the assistant in place of the microphone and says how the turn went: {passed, phase, heard, reply, error, tookMs}; loopback only, and answered only while helper.json's voiceAssistant.selftest is true",
)];
pub const OPT_IN: OptIn = OptIn {
    spec: Spec {
        id: ID,
        name: "Voice assistant",
        version: "1",
        permissions: &["settings"],
    },
    consent: "Lets this TV listen for a wake word and take what is said after it to Home Assistant. While this is on and the TV is paired \
              with Home Assistant, the microphone is open: the TV listens for the wake word itself, and nothing is sent or kept until it \
              hears it. What is said after it goes to Home Assistant, which answers; the TV shows both on the screen and speaks the answer, \
              except from 01:00 to 07:00 (changeable on this page), when it only shows it. The TV also shows what other Home Assistant \
              assistants in the same room hear and say. Turning this off closes the microphone.",
};

/// The pace of the thread while it is on and the host has something to wait on: the host's step is the wait.
const STEP: Duration = Duration::from_millis(250);
/// The pace while it is on and cannot step yet (no address, no host).
const IDLE_STEP: Duration = Duration::from_secs(1);
/// The longest the thread sleeps while the plugin is off.
const LONGEST_SLEEP: Duration = Duration::from_secs(3600);
/// How often the TV's address is looked for.
const NET_LOOK: Duration = Duration::from_secs(5);
/// The pause before the microphone, the host or the announcement is tried again after it failed.
const RETRY: Duration = Duration::from_secs(10);
/// How often the audio processor's reset count is read.
const GUARD_EVERY: Duration = Duration::from_secs(5);
const DSP_RESETS: &str = "/sys/module/aud/parameters/aud_dsp_reset_count";
/// The TV's microphone, opened at the format and period the card needs (`plughw` on it is refused).
const MIC_DEVICE: &str = "hw:1,0";
/// The microphone's ALSA buffer in frames. With 2048 the driver's position runs past the buffer and the capture stops delivering
/// after about 3 s (measured 2026-11-23); 8192 ran 10 minutes.
const MIC_BUFFER: u32 = 8192;
/// The TV's mixed output, the echo canceller's reference.
const REFERENCE_DEVICE: &str = "hw:1,2";
/// The player of replies: raw 16-bit mono, the rate is added. `media` is the TV's mixed output.
const PLAYER: [&str; 10] = ["aplay", "-q", "-D", "plug:media", "-t", "raw", "-f", "S16_LE", "-c", "1"];
const SOCKET: &str = "audiod.sock";
const SELFTEST_CLIP: &str = "selftest.wav";
/// How long the self-check waits for the turn to end.
const SELFTEST_WITHIN: Duration = Duration::from_secs(30);
const STACK: usize = 1024 * 1024;
/// How the page reads when the plugin has not been told more.
const DEFAULT_ROOM: &str = "Living Room";
const DEFAULT_VOLUME: u8 = 50;
const DEFAULT_FROM: u8 = 1;
const DEFAULT_UNTIL: u8 = 7;

/// The page, as the schema format writes it.
fn schema_json() -> Json {
    let read = |key: &str, label: &str, help: &str| {
        json!({"key": key, "label": label, "help": help,
               "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.{key}")}}, "apply": "on-commit"})
    };
    let choice = |key: &str, label: &str, help: &str, options: &[(&str, &str)], default: &str| {
        let inline: Vec<Json> = options.iter().map(|(id, label)| json!({"id": id, "label": label})).collect();
        json!({"key": key, "label": label, "help": help, "kind": {"type": "choice", "options": {"inline": inline}},
               "default": {"type": "choice", "value": default}, "apply": "on-commit"})
    };
    let hour = |key: &str, label: &str, default: u8| {
        json!({"key": key, "label": label, "kind": {"type": "integer", "min": 0, "max": 23, "step": 1, "unit": "h"},
               "default": {"type": "integer", "value": default}, "apply": "on-commit",
               "enabled_when": {"type": "equals", "key": "quiet-hours", "value": {"type": "bool", "value": true}}})
    };
    json!({"format": 1, "owner": ID, "revision": 1, "title": OPT_IN.spec.name, "sections": [
        {"id": "assistant", "title": OPT_IN.spec.name, "fields": [
            read("status", "Status", "How the assistant stands."),
            read("microphone", "Microphone", "Open only while this is on and the TV is paired with Home Assistant."),
            read("homeassistant", "Home Assistant", "Whether the TV is paired and Home Assistant is connected."),
            read("last", "Last request", "How the last request went and when. What was said is never kept."),
            read("others", "Other assistants", "What the assistants in the room below do, as Home Assistant tells it."),
            choice("wake-word", "Wake word", "What to say. Okay Nabu leaves the other assistants, which answer to Hey Jarvis, alone.",
                   &[("okay_nabu", "Okay Nabu"), ("hey_jarvis", "Hey Jarvis")], "okay_nabu"),
            choice("sensitivity", "Sensitivity", "Strict hears the wake word less often and is falsely woken less.",
                   &[("strict", "Strict"), ("normal", "Normal"), ("lenient", "Lenient")], "normal"),
            {"key": "speak", "label": "Speak answers", "help": "Off shows the answer on the screen and says nothing.",
             "kind": {"type": "bool"}, "default": {"type": "bool", "value": true}, "apply": "on-commit"},
            {"key": "volume", "label": "Answer volume",
             "kind": {"type": "integer", "min": 5, "max": 100, "step": 5, "unit": "%"},
             "default": {"type": "integer", "value": DEFAULT_VOLUME}, "apply": "on-commit"},
            {"key": "room", "label": "Room",
             "help": "The Home Assistant area of this TV: the assistants there are shown on the screen when they listen and answer. Empty shows none.",
             "kind": {"type": "text", "max_chars": 60, "secret": false},
             "default": {"type": "text", "value": DEFAULT_ROOM}, "apply": "on-commit"},
            {"key": "quiet-hours", "label": "Text only at night",
             "help": "Answers are shown and not spoken between these hours.",
             "kind": {"type": "bool"}, "default": {"type": "bool", "value": true}, "apply": "on-commit"},
            hour("quiet-from", "From", DEFAULT_FROM),
            hour("quiet-until", "Until", DEFAULT_UNTIL),
            {"key": "pair", "label": "Pair with Home Assistant",
             "help": "Opens a window of five minutes in which Home Assistant can add this TV (Settings, Devices, Add integration, ESPHome).",
             "kind": {"type": "action", "action": "pair", "confirmation": "confirm", "verb": "Pair"}, "apply": "on-commit"},
            {"key": "forget", "label": "Forget Home Assistant",
             "help": "Closes the microphone and forgets the pairing. Remove the TV in Home Assistant too.",
             "kind": {"type": "action", "action": "forget", "confirmation": "destructive", "role": "danger"}, "apply": "on-commit"}]}]})
}

fn register() -> Json {
    json!({"title": OPT_IN.spec.name, "settings": schema_json(), "actions": [
        {"key": "pair", "label": "Pair with Home Assistant", "confirmation": "confirm",
         "confirm_text": {"title": "Pair with Home Assistant?", "confirm_label": "Pair",
                          "body": "For five minutes the TV shows itself to Home Assistant, which can then add it. A pairing the TV has already is forgotten first."}},
        {"key": "forget", "label": "Forget Home Assistant", "confirmation": "destructive",
         "confirm_text": {"title": "Forget Home Assistant?", "confirm_label": "Forget",
                          "body": "The microphone closes and the TV forgets the pairing. Remove the TV in Home Assistant too."}}]})
}

/// What the owner's page asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Choices {
    word: &'static str,
    sensitivity: Sensitivity,
    speak: bool,
    volume: u8,
    room: String,
    /// The hours between which it only shows its answers (from, until), when it is to.
    quiet: Option<(u8, u8)>,
}

impl Choices {
    /// The page's values, each as the page's default where it has none.
    fn read(values: &Values) -> Self {
        let word = match values.get("wake-word") {
            Some(Setting::Choice(id)) => models::find(&id.to_string()).map(|w| w.id),
            _ => None,
        }
        .unwrap_or(models::WORDS[0].id);
        let sensitivity = match values.get("sensitivity") {
            Some(Setting::Choice(id)) => Sensitivity::parse(&id.to_string()),
            _ => None,
        }
        .unwrap_or(Sensitivity::Normal);
        let number = |key: &str, least: i64, most: i64, default: u8| match values.get(key) {
            Some(Setting::Integer(n)) => u8::try_from((*n).clamp(least, most)).unwrap_or(default),
            _ => default,
        };
        let room = match values.get("room") {
            Some(Setting::Text(room)) => room.trim().to_owned(),
            _ => DEFAULT_ROOM.to_owned(),
        };
        let quiet = (!matches!(values.get("quiet-hours"), Some(Setting::Bool(false))))
            .then(|| (number("quiet-from", 0, 23, DEFAULT_FROM), number("quiet-until", 0, 23, DEFAULT_UNTIL)));
        Self {
            word,
            sensitivity,
            speak: !matches!(values.get("speak"), Some(Setting::Bool(false))),
            volume: number("volume", 1, 100, DEFAULT_VOLUME),
            room,
            quiet,
        }
    }
}

/// Whether `hour` (the TV's own, `None` when it cannot be told) is within the quiet hours. From before until: the hours between; from
/// after until: the night that wraps midnight; the same hour twice: all day. A TV that does not know the hour is quiet.
fn is_quiet(quiet: Option<(u8, u8)>, hour: Option<u32>) -> bool {
    let Some((from, until)) = quiet else { return false };
    let Some(hour) = hour else { return true };
    let (from, until) = (u32::from(from), u32::from(until));
    match from.cmp(&until) {
        std::cmp::Ordering::Less => from <= hour && hour < until,
        std::cmp::Ordering::Greater => hour >= from || hour < until,
        std::cmp::Ordering::Equal => true,
    }
}

fn quiet_now(quiet: Option<(u8, u8)>, unix_s: i64) -> bool {
    is_quiet(quiet, local_time(unix_s).map(|t| t.hour))
}

/// How long ago, in the page's words.
fn ago(age: Duration) -> String {
    match age.as_secs() {
        0..=59 => "just now".to_owned(),
        60..=3599 => format!("{} min ago", age.as_secs() / 60),
        3600..=86_399 => format!("{} h ago", age.as_secs() / 3600),
        s => format!("{} d ago", s / 86_400),
    }
}

/// Everything the page's facts say, as numbers and words: none of what was said.
#[derive(Clone, Debug, Default)]
struct View {
    paired: bool,
    /// The minutes the window to pair has left, while it is open.
    window_min: Option<u64>,
    network: bool,
    phrase: String,
    /// Why the plugin stopped itself.
    stopped: Option<String>,
    /// The sound service's status while it runs.
    engine: Option<Status>,
    /// Why the sound service could not start.
    engine_error: Option<String>,
    connected: bool,
    /// Whether answers are shown and not spoken.
    text_only: bool,
    /// How long ago the last turn ended, and whether it worked.
    last: Option<(Duration, bool)>,
    others: String,
}

impl View {
    fn status(&self) -> String {
        if !self.network {
            return "Waiting for the network".to_owned();
        }
        if let Some(why) = &self.stopped {
            return format!("Stopped: {why}");
        }
        if !self.paired {
            return match self.window_min {
                Some(min) => format!("Ready to pair: add the TV in Home Assistant ({min} min left)"),
                None => "Not paired".to_owned(),
            };
        }
        if let Some(why) = &self.engine_error {
            return format!("The microphone did not start: {why}");
        }
        let Some(engine) = &self.engine else { return "Starting".to_owned() };
        if engine.in_turn {
            return "Talking with Home Assistant".to_owned();
        }
        let base = if self.connected {
            format!("Listening for {}", self.phrase)
        } else {
            "Paired, waiting for Home Assistant".to_owned()
        };
        if self.text_only { format!("{base}, text only") } else { base }
    }

    fn microphone(&self) -> String {
        let Some(engine) = &self.engine else { return "Closed".to_owned() };
        if let Some(why) = &engine.error {
            format!("Not working: {why}")
        } else if !engine.capturing {
            "Opening".to_owned()
        } else if engine.silent {
            "Open, but it sends no sound".to_owned()
        } else if engine.in_turn {
            "In use: sending what is said to Home Assistant".to_owned()
        } else {
            "Open, listening for the wake word".to_owned()
        }
    }

    fn home_assistant(&self) -> String {
        match (self.paired, self.connected, self.window_min) {
            (true, true, _) => "Connected to Home Assistant".to_owned(),
            (true, false, _) => "Paired, not connected".to_owned(),
            (false, _, Some(_)) => "Waiting for Home Assistant to add the TV".to_owned(),
            (false, _, None) => "Not paired".to_owned(),
        }
    }

    fn last(&self) -> String {
        match self.last {
            None => "None yet".to_owned(),
            Some((age, true)) => format!("Answered {}", ago(age)),
            Some((age, false)) => format!("Did not work {}", ago(age)),
        }
    }

    fn facts(&self) -> Map<String, Json> {
        let line = |text: String| json!(clip(&text.replace(char::is_control, " "), MAX_FACT_LINE));
        let mut facts = Map::new();
        facts.insert("status".into(), line(self.status()));
        facts.insert("microphone".into(), line(self.microphone()));
        facts.insert("homeassistant".into(), line(self.home_assistant()));
        facts.insert("last".into(), line(self.last()));
        facts.insert("others".into(), line(self.others.clone()));
        facts
    }
}

/// What the screen is told: one document, kept up to date by every side that has something to say about a turn (the voice host, and the
/// watcher of the other assistants in the room).
pub struct Surface {
    hub: Arc<Hub>,
    overlay: Mutex<Overlay>,
}

impl Surface {
    fn new(hub: Arc<Hub>) -> Arc<Self> {
        let surface = Arc::new(Self {
            hub,
            overlay: Mutex::new(Overlay::new(now_ms())),
        });
        surface.with(|_, _| {});
        surface
    }

    /// Changes the overlay and publishes it; the lock is held through the publish so that two sides cannot publish out of order.
    fn with(&self, change: impl FnOnce(&mut Overlay, Instant)) {
        let mut overlay = self.overlay.lock();
        change(&mut overlay, Instant::now());
        let _ = self.hub.set(TOPIC, overlay.doc());
    }

    /// What another assistant in the room does.
    fn other(&self, label: &str, event: &Other) {
        self.with(|overlay, now| overlay.other(label, event, now));
    }

    fn shown(&self, shown: &Shown) {
        self.with(|overlay, now| overlay.shown(shown, now));
    }

    fn set_enabled(&self, on: bool) {
        self.with(|overlay, _| overlay.set_enabled(on));
    }

    fn set_mic(&self, mic: Mic) {
        self.with(|overlay, _| overlay.set_mic(mic));
    }

    fn tick(&self) {
        self.with(|overlay, now| {
            overlay.tick(now);
        });
    }
}

/// The host's surroundings: what it shows goes to the screen, its key to the file, and the rest is remembered for the page.
struct TvEnv {
    surface: Arc<Surface>,
    keys: KeyFile,
    /// The key Home Assistant set (all zeros: none).
    key: [u8; 32],
    /// A key was given since the loop last looked.
    new_key: bool,
    /// Home Assistant is subscribed.
    connected: bool,
    /// The level Home Assistant set, since the loop last looked.
    volume: Option<u8>,
    /// Whether the turn that is going worked, once it has an answer.
    turn_ok: Option<bool>,
    /// When the last turn ended, and whether it worked.
    last: Option<(Instant, bool)>,
}

impl TvEnv {
    fn new(surface: Arc<Surface>, keys: KeyFile) -> Self {
        Self {
            surface,
            keys,
            key: [0; 32],
            new_key: false,
            connected: false,
            volume: None,
            turn_ok: None,
            last: None,
        }
    }

    fn keyed(&self) -> bool {
        self.key != [0; 32]
    }
}

impl Env for TvEnv {
    fn command(&mut self, _text: &str) -> voice_intent::Reply {
        voice_intent::Reply {
            text: "I can't do that here.".to_owned(),
            ok: false,
        }
    }

    fn show(&mut self, shown: &Shown) {
        match shown {
            Shown::Reply { ok, .. } => self.turn_ok = Some(*ok),
            Shown::Idle => {
                if let Some(ok) = self.turn_ok.take() {
                    self.last = Some((Instant::now(), ok));
                }
            }
            _ => {}
        }
        self.surface.shown(shown);
    }

    /// The answer is on the screen, and a chime would be a sound where the owner asked for none.
    fn chime(&mut self, _chime: Chime) {}

    fn satellite(&mut self, effect: Effect) {
        match effect {
            Effect::Connected => self.connected = true,
            Effect::Disconnected => self.connected = false,
            _ => {}
        }
    }

    fn paired(&mut self, key: [u8; 32]) {
        if let Err(e) = self.keys.save(&key) {
            crate::log_warn!("voice-assistant: the pairing key was not kept: {e}");
        }
        self.key = key;
        self.new_key = true;
    }

    fn set_volume(&mut self, percent: u8) {
        self.volume = Some(percent.clamp(1, 100));
    }
}

/// Watches the audio processor's reset count: LG's audio processor resets when something touches it the wrong way, and a microphone
/// that does it must not stay open.
struct DspGuard {
    path: PathBuf,
    /// The count when the microphone was opened; none while it is closed.
    baseline: Option<u64>,
    next: Instant,
}

impl DspGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            baseline: None,
            next: Instant::now(),
        }
    }

    fn read(&self) -> Option<u64> {
        std::fs::read_to_string(&self.path).ok()?.trim().parse().ok()
    }

    /// The microphone is about to be opened: holds the processor to the count it has now.
    fn arm(&mut self) {
        self.baseline = self.read();
        self.next = Instant::now() + GUARD_EVERY;
    }

    fn disarm(&mut self) {
        self.baseline = None;
    }

    /// Whether the count has risen since [`arm`](Self::arm); looked at every few seconds.
    fn tripped(&mut self, now: Instant) -> bool {
        let Some(baseline) = self.baseline else { return false };
        if now < self.next {
            return false;
        }
        self.next = now + GUARD_EVERY;
        self.read().is_some_and(|count| count > baseline)
    }
}

enum Event {
    /// The owner's switch may have changed: [`Shared::switched_on`] says how it stands.
    Switch,
    Settings(Values),
    Pair,
    Forget,
    Stop,
}

/// Where the TV can be reached, and what it calls itself to Home Assistant.
struct Net {
    ip: Ipv4Addr,
    identity: Identity,
}

fn find_net() -> Option<Net> {
    let ip = crate::sys::lan_ipv4(None)?;
    let mac = pair::device_mac(&interface_mac(ip)?)?;
    Some(Net {
        ip,
        identity: pair::identity(&mac),
    })
}

/// What the self-check needs of the running sound service.
struct Door {
    engine: Arc<Handle>,
    hub: Arc<Hub>,
    /// helper.json's `voiceAssistant.selftest`.
    allowed: bool,
    clip: PathBuf,
}

/// The sound service while it runs.
static DOOR: RwLock<Option<Arc<Door>>> = RwLock::new(None);

/// Offers the plugin (off until the owner switches it on) and starts its thread.
pub fn spawn(ctx: &Ctx, shared: Arc<Shared>) -> io::Result<JoinHandle<()>> {
    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        ctx.stop.on_stop(move || {
            let _ = tx.send(Event::Stop);
        });
    }
    let mut plugin = Plugin::new(ctx, shared.clone(), tx.clone());
    shared.offer(
        &OPT_IN,
        Arc::new(move || {
            let _ = tx.send(Event::Switch);
        }),
    );
    crate::plugins::guard::spawn("voice-assistant", STACK, shared, &[ID], ctx.stop.clone(), move |again| {
        crate::sys::bulk_thread();
        if again {
            plugin.follow_afresh();
        }
        plugin.run(&rx)
    })
}

struct Plugin {
    ctx: Ctx,
    shared: Arc<Shared>,
    tx: Sender<Event>,
    /// `voice-assistant` in the data directory: the key, the sound service's socket, the self-check's clip.
    dir: PathBuf,
    keys: KeyFile,
    surface: Arc<Surface>,
    env: TvEnv,
    /// Its conversation with the broker's session while it is on: its page and its facts.
    page: Option<Arc<Local>>,
    on: bool,
    choices: Choices,
    /// The facts last put: unchanged ones are not put again.
    said: Json,
    net: Option<Net>,
    next_net: Instant,
    host: Option<Host>,
    next_host: Instant,
    /// What the host was last told about speaking.
    spoken: Option<(bool, u8)>,
    window: Window,
    announce: Announcement<MdnsAnnouncer>,
    next_announce: Instant,
    engine: Option<Arc<Handle>>,
    engine_error: Option<String>,
    next_engine: Instant,
    /// The sound service's status as the loop last read it.
    mic: Option<Status>,
    guard: DspGuard,
    /// Why the plugin stopped itself, until it is switched off and on.
    stopped: Option<String>,
    /// The watcher of the other assistants in the room, and the room it was made for.
    follower: Option<(Follower, String)>,
    quiet: bool,
    /// What the helper's log was told last: a problem that lasts is said once.
    logged: String,
}

impl Plugin {
    fn new(ctx: &Ctx, shared: Arc<Shared>, tx: Sender<Event>) -> Self {
        let now = Instant::now();
        let dir = ctx.paths.data_dir.join(ID);
        let surface = Surface::new(ctx.hub.clone());
        Self {
            ctx: ctx.clone(),
            shared,
            tx,
            keys: KeyFile::new(dir.join("key")),
            env: TvEnv::new(surface.clone(), KeyFile::new(dir.join("key"))),
            surface,
            dir,
            page: None,
            on: false,
            choices: Choices::read(&Values::new()),
            said: Json::Null,
            net: None,
            next_net: now,
            host: None,
            next_host: now,
            spoken: None,
            window: Window::default(),
            announce: Announcement::new(MdnsAnnouncer::default()),
            next_announce: now,
            engine: None,
            engine_error: None,
            next_engine: now,
            mic: None,
            guard: DspGuard::new(PathBuf::from(DSP_RESETS)),
            stopped: None,
            follower: None,
            quiet: false,
            logged: String::new(),
        }
    }

    /// After a panic: follows the switch from scratch, everything being as the panic left it.
    fn follow_afresh(&mut self) {
        self.on = !self.shared.switched_on(ID);
        self.switch();
    }

    /// Whether the host can be stepped: it exists and the TV has an address.
    fn stepping(&self) -> bool {
        self.host.is_some() && self.net.is_some()
    }

    fn run(&mut self, rx: &Receiver<Event>) {
        loop {
            // The host's step is the wait while it runs; otherwise the thread waits for an event.
            let wait = if !self.on {
                LONGEST_SLEEP
            } else if self.stepping() {
                Duration::ZERO
            } else {
                IDLE_STEP
            };
            let first = match rx.recv_timeout(wait) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            };
            let mut stop = false;
            for event in first.into_iter().chain(rx.try_iter()) {
                match event {
                    Event::Stop => {
                        stop = true;
                        break;
                    }
                    Event::Switch => self.switch(),
                    Event::Settings(values) => self.settings(&values),
                    Event::Pair => self.pair(),
                    Event::Forget => self.forget(),
                }
            }
            if stop || self.ctx.stop.is_set() {
                break;
            }
            if self.on {
                self.work();
            }
            self.publish();
        }
        self.halt();
        self.page = None;
        self.surface.set_enabled(false);
    }

    /// Follows the owner's switch: on registers the page and, once paired, opens the microphone; off closes everything and takes the page
    /// away.
    fn switch(&mut self) {
        let on = self.shared.switched_on(ID);
        if on == self.on {
            return;
        }
        self.on = on;
        self.halt();
        if on {
            match Local::start(&self.shared, &OPT_IN.spec, register(), handler(self.tx.clone())) {
                Ok(page) => self.page = Some(page),
                Err(e) => crate::log_warn!("voice-assistant: the plugin's page was not registered: {e}"),
            }
            self.choices = Choices::read(&self.ctx.settings.effective(ID).unwrap_or_default());
            self.env.key = self.keys.load();
            self.stopped = None;
            self.engine_error = None;
            let now = Instant::now();
            (self.next_net, self.next_host, self.next_engine, self.next_announce) = (now, now, now, now);
            self.surface.set_enabled(true);
            crate::log_info!(
                "voice-assistant: on, {}",
                if self.env.keyed() { "paired" } else { "not paired" }
            );
        } else {
            self.page = None;
            self.said = Json::Null;
            self.surface.set_enabled(false);
            crate::log_info!("voice-assistant: off, the microphone is closed");
        }
    }

    /// Takes the page's values; what the sound service or the host was built from is built again.
    fn settings(&mut self, values: &Values) {
        if !self.on {
            return;
        }
        let new = Choices::read(values);
        let old = std::mem::replace(&mut self.choices, new);
        if old.word != self.choices.word {
            self.stop_engine();
            self.host = None;
        } else if old.sensitivity != self.choices.sensitivity {
            self.stop_engine();
        }
        if old.room != self.choices.room {
            self.follower = None;
        }
        self.spoken = None;
    }

    /// Opens the window in which Home Assistant may add the TV. A TV that is paired forgets that first: only a TV without a key lets
    /// Home Assistant make one.
    fn pair(&mut self) {
        if !self.on {
            return;
        }
        self.drop_key();
        self.window.open(Instant::now());
        crate::log_info!("voice-assistant: the window to pair is open");
    }

    fn forget(&mut self) {
        if !self.on {
            return;
        }
        self.drop_key();
        self.window.close();
        crate::log_info!("voice-assistant: the pairing is forgotten");
    }

    fn drop_key(&mut self) {
        self.keys.forget();
        self.env.key = [0; 32];
        if let Some(host) = self.host.as_mut() {
            host.set_satellite_key([0; 32], &mut self.env);
        }
    }

    /// Closes everything the plugin runs; the page and the switch are the caller's.
    fn halt(&mut self) {
        self.follower = None;
        self.announce.withdraw();
        self.stop_engine();
        self.host = None;
        self.window.close();
        self.mic = None;
        self.env.connected = false;
        self.surface.set_mic(Mic::Off);
    }

    fn work(&mut self) {
        let now = Instant::now();
        self.look_net(now);
        self.keep_follower();
        if self.net.is_some() {
            if self.host.is_none() {
                self.build_host(now);
            }
            self.keep_window(now);
            self.keep_engine(now);
            self.keep_voice();
            if let Some(host) = self.host.as_mut() {
                host.step(&mut self.env, STEP);
            }
            self.keep_announcement(now);
        }
        self.after_step(now);
    }

    fn look_net(&mut self, now: Instant) {
        if now < self.next_net {
            return;
        }
        self.next_net = now + NET_LOOK;
        self.net = find_net();
    }

    fn keep_follower(&mut self) {
        let room = self.choices.room.clone();
        let Some(net) = &self.net else {
            self.follower = None;
            return;
        };
        if room.is_empty() {
            self.follower = None;
            return;
        }
        if self.follower.as_ref().is_some_and(|(_, made_for)| *made_for == room) {
            return;
        }
        self.follower = None;
        let params = Params {
            links: vec![self.ctx.paths.ha_link_file(), self.ctx.paths.data_dir.join("voice").join("ha.json")],
            tls: crate::voice::tls_for(&self.ctx.config.fetch),
            room: room.clone(),
            own_mac: net.identity.mac.clone(),
        };
        match Follower::spawn(params, self.surface.clone()) {
            Ok(follower) => self.follower = Some((follower, room)),
            Err(e) => crate::log_warn!("voice-assistant: the watcher of the room's assistants did not start: {e}"),
        }
    }

    fn build_host(&mut self, now: Instant) {
        if now < self.next_host {
            return;
        }
        let Some(net) = &self.net else { return };
        let info = models::find(self.choices.word).unwrap_or(&models::WORDS[0]);
        let settings = Settings {
            enabled: false,
            mode: Mode::HomeAssistant,
            words: vec![WakeWord {
                id: info.id.to_owned(),
                phrase: info.phrase.to_owned(),
            }],
            satellite_bind: format!("0.0.0.0:{}", self.ctx.config.voice_assistant.port),
            satellite_key: self.env.key,
            pairing_open: self.window.is_open(),
            satellite_name: net.identity.name.clone(),
            satellite_mac: net.identity.mac.clone(),
            arbitration: Arbitration::Off,
            device_id: pair::device_number(&net.identity.mac),
            speak: false,
            volume: self.choices.volume,
        };
        let satellite = Satellite {
            identity: net.identity.clone(),
            wake_words: vec![WakeWordInfo {
                id: info.id.to_owned(),
                phrase: info.phrase.to_owned(),
                languages: vec!["en".to_owned()],
            }],
            active_wake_words: vec![info.id.to_owned()],
            max_active_wake_words: 1,
            reply_wait_s: 30.0,
            stream_reply: true,
        };
        let services = Services {
            tls: crate::voice::tls_for(&self.ctx.config.fetch),
            ..Services::default()
        };
        match Host::new(settings, satellite, self.dir.join(SOCKET), services) {
            Ok(host) => {
                self.host = Some(host);
                self.spoken = None;
            }
            Err(e) => {
                self.next_host = now + RETRY;
                self.note(&format!("the voice host did not start: {e}"));
            }
        }
    }

    /// The window to pair, as the host has it.
    fn keep_window(&mut self, now: Instant) {
        if self.window.ran_out(now) {
            crate::log_info!("voice-assistant: the window to pair ran out");
        }
        let open = self.window.is_open();
        if let Some(host) = self.host.as_mut() {
            let differs = host.settings().pairing_open != open;
            if differs {
                host.set_pairing_open(open, &mut self.env);
            }
        }
    }

    /// The sound service runs while the TV is paired, and not otherwise.
    fn keep_engine(&mut self, now: Instant) {
        let want = self.env.keyed() && self.stopped.is_none();
        if want && self.engine.is_none() && now >= self.next_engine {
            match self.start_engine() {
                Ok(engine) => {
                    self.engine_error = None;
                    self.guard.arm();
                    *DOOR.write() = Some(Arc::new(Door {
                        engine: engine.clone(),
                        hub: self.ctx.hub.clone(),
                        allowed: self.ctx.config.voice_assistant.selftest,
                        clip: self.dir.join(SELFTEST_CLIP),
                    }));
                    self.engine = Some(engine);
                    crate::log_info!("voice-assistant: the microphone is open");
                }
                Err(why) => {
                    self.next_engine = now + RETRY;
                    self.note(&format!("the microphone did not start: {why}"));
                    self.engine_error = Some(why);
                }
            }
        } else if !want && self.engine.is_some() {
            self.stop_engine();
            crate::log_info!("voice-assistant: the microphone is closed");
        }
        let open = self.engine.is_some();
        if let Some(host) = self.host.as_mut() {
            if host.settings().enabled != open {
                host.set_enabled(open);
            }
        }
    }

    fn start_engine(&self) -> Result<Arc<Handle>, String> {
        pair::private_dir(&self.dir).map_err(|e| format!("{}: {e}", self.dir.display()))?;
        // The mix capture is asked for 48 kHz, the only rate it accepts, but its frames come at twice that pace (measured
        // 2026-11-23: 10 s of them in 5 s), so it is read as 96 kHz. Only its right channel carries the TV's sound.
        let asked = Format {
            rate: 48_000,
            channels: 2,
            sample: Sample::S16,
        };
        let card = Alsa {
            mic: Alsa::arecord(MIC_DEVICE, tv_mic_format(), 256, MIC_BUFFER),
            mic_format: tv_mic_format(),
            reference: Alsa::arecord(REFERENCE_DEVICE, asked, 0, 0),
            reference_format: Format { rate: 96_000, ..asked },
        };
        let speaker = Aplay::new(PLAYER.iter().map(|s| (*s).to_owned()).collect());
        let config = lumavoice::Config {
            cutoff: self.choices.sensitivity.cutoff(),
            reference_pick: lumavoice::Pick::Channel(1),
            ..lumavoice::Config::default()
        };
        lumavoice::start(&self.dir.join(SOCKET), Arc::new(card), Box::new(speaker), config)
            .map(Arc::new)
            .map_err(|e| e.to_string())
    }

    /// Releases the microphone: the host is told first so that it lets go of the service, and the service ends when the last holder
    /// of it lets go.
    fn stop_engine(&mut self) {
        *DOOR.write() = None;
        self.guard.disarm();
        if self.engine.is_none() {
            return;
        }
        if let Some(host) = self.host.as_mut() {
            host.set_enabled(false);
            host.step(&mut self.env, Duration::from_millis(20));
        }
        self.engine = None;
        self.mic = None;
    }

    /// Whether the voice speaks, and how loudly: the page's choice, and not in quiet hours.
    fn keep_voice(&mut self) {
        let unix = i64::try_from(now_ms() / 1000).unwrap_or(0);
        self.quiet = quiet_now(self.choices.quiet, unix);
        let speak = self.choices.speak && !self.quiet;
        let spoken = Some((speak, self.choices.volume));
        if self.spoken != spoken {
            if let Some(host) = self.host.as_mut() {
                host.set_speech(speak, self.choices.volume);
                self.spoken = spoken;
            }
        }
    }

    /// The announcement Home Assistant finds the TV by: while the TV can be added (the window is open) or is paired.
    fn keep_announcement(&mut self, now: Instant) {
        if now < self.next_announce {
            return;
        }
        let wanted = match (&self.host, &self.net) {
            (Some(host), Some(net)) => host.satellite_addr().and_then(|addr| match host.access() {
                Access::Keyed(_) => Some((net.ip, true, addr.port())),
                Access::Open => Some((net.ip, false, addr.port())),
                Access::Closed => None,
            }),
            _ => None,
        };
        let (Some(net), port) = (&self.net, wanted.map_or(0, |(_, _, port)| port)) else { return };
        if let Err(e) = self.announce.keep(wanted.map(|(ip, paired, _)| (ip, paired)), &net.identity, port) {
            self.next_announce = now + RETRY;
            let words = format!("the TV was not announced to Home Assistant: {e}");
            self.note(&words);
        }
    }

    /// What the loop does after each step: the key the host was given, the level Home Assistant set, the screen, the guard.
    fn after_step(&mut self, now: Instant) {
        if std::mem::take(&mut self.env.new_key) {
            self.window.close();
            crate::log_info!("voice-assistant: paired with Home Assistant");
        }
        if let Some(volume) = self.env.volume.take() {
            self.choices.volume = volume;
        }
        self.mic = self.engine.as_ref().map(|engine| engine.status());
        let mic = match &self.mic {
            Some(status) if status.capturing && status.in_turn => Mic::Turn,
            Some(status) if status.capturing => Mic::Listening,
            _ => Mic::Off,
        };
        self.surface.set_mic(mic);
        self.surface.tick();
        if self.engine.is_some() && self.guard.tripped(now) {
            self.stop_for("LG's audio processor reset while the microphone was open");
        }
    }

    /// Closes the microphone and keeps it closed until the owner switches the plugin off and on.
    fn stop_for(&mut self, why: &str) {
        self.note(why);
        self.stopped = Some(why.to_owned());
        self.stop_engine();
    }

    fn note(&mut self, words: &str) {
        if self.logged != words {
            crate::log_warn!("voice-assistant: {words}");
            self.logged = words.to_owned();
        }
    }

    fn view(&self, now: Instant) -> View {
        let phrase = models::find(self.choices.word).map_or(self.choices.word, |w| w.phrase);
        View {
            paired: self.env.keyed(),
            window_min: self.window.left(now).map(|left| left.as_secs().div_ceil(60)),
            network: self.net.is_some(),
            phrase: phrase.to_owned(),
            stopped: self.stopped.clone(),
            engine: self.mic.clone(),
            engine_error: self.engine_error.clone(),
            connected: self.env.connected,
            text_only: !self.choices.speak || self.quiet,
            last: self.env.last.map(|(at, ok)| (now.saturating_duration_since(at), ok)),
            others: match (&self.follower, self.choices.room.is_empty()) {
                (Some((follower, _)), _) => follower.said(),
                (None, true) => "Off: no room is set".to_owned(),
                (None, false) => "Starting".to_owned(),
            },
        }
    }

    /// Puts the page's facts when they changed.
    fn publish(&mut self) {
        let Some(page) = &self.page else { return };
        let said = json!({"t": "facts.put", "facts": self.view(Instant::now()).facts()});
        if said != self.said {
            if let Err(e) = page.say(said.clone()) {
                crate::log_warn!("voice-assistant: the page's facts were not put: {e}");
            }
            self.said = said;
        }
    }
}

/// Takes what the base says to the thread: the page's values, and the two actions.
fn handler(tx: Sender<Event>) -> Handler {
    Arc::new(move |local: &Local, message: ToPlugin| match message {
        ToPlugin::SettingsApply { txn, values, .. } => {
            let applied = json!({"t": "settings.result", "txn": txn, "outcome": {"type": "applied", "values": values}});
            let _ = tx.send(Event::Settings(values));
            let _ = local.say(applied);
        }
        ToPlugin::ActionInvoke { op, action, .. } => {
            let event = match action.as_str() {
                "pair" => Some(Event::Pair),
                "forget" => Some(Event::Forget),
                _ => None,
            };
            match event {
                Some(event) => {
                    let _ = tx.send(event);
                    let _ = local.answer(op, Ok(()));
                }
                None => {
                    let _ = local.answer(op, Err("no such action"));
                }
            }
        }
        _ => {}
    })
}

/// Serves `/v1/voice-assistant/<op>`.
pub fn handle(request: &Request, op: &str) -> Response {
    if !request.peer.is_loopback() {
        return error_response(403, "local_only", "the voice assistant's self-check is for a program on the TV itself");
    }
    if op != "selftest" {
        return error_response(404, "not_found", "there is no such voice assistant call");
    }
    if request.method != "POST" {
        return error_response(405, "method_not_allowed", "this call takes POST").with_header("Allow", "POST");
    }
    match DOOR.read().clone() {
        None => error_response(
            409,
            "not_running",
            "the microphone is not open: the assistant is off or not paired",
        ),
        Some(door) if !door.allowed => error_response(
            403,
            "selftest_off",
            "the self-check is off: helper.json's voiceAssistant.selftest allows it",
        ),
        Some(door) => selftest(door),
    }
}

/// Feeds the clip in place of the microphone and waits for the turn it starts to end.
fn selftest(door: Arc<Door>) -> Response {
    let samples = match read_clip(&door.clip) {
        Ok(samples) => samples,
        Err(why) => return error_response(409, "no_clip", &why),
    };
    let seen = |hub: &Hub| {
        let doc = hub.get(TOPIC).unwrap_or_default();
        // another assistant's turn carries a source; the TV's own does not
        let own = doc.get("source").is_none();
        (doc["turn"].clone(), own)
    };
    let hub = door.hub.clone();
    let before = seen(&hub).0["id"].as_u64();
    let began = Instant::now();
    if let Err(why) = door.engine.inject(samples) {
        return error_response(409, "not_ready", &why);
    }
    // the service is the plugin's to close: nothing here holds it while the turn runs
    drop(door);
    let mut turn = Json::Null;
    while began.elapsed() < SELFTEST_WITHIN {
        std::thread::sleep(Duration::from_millis(100));
        let (now, own) = seen(&hub);
        if own && now["id"].as_u64() > before {
            let done = matches!(now["phase"].as_str(), Some("done" | "failed"));
            turn = now;
            if done {
                break;
            }
        }
    }
    let phase = turn["phase"].as_str().unwrap_or("none");
    let ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
    Response::json(
        200,
        &json!({"ok": true, "api": API_VERSION, "passed": phase == "done", "phase": phase,
                "heard": turn["heard"], "reply": turn["reply"], "error": turn["error"], "tookMs": ms}),
    )
}

/// The clip the self-check plays: a 16 kHz mono 16-bit WAV.
fn read_clip(path: &Path) -> Result<Vec<i16>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let wav = wake_word::wav::read(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if wav.rate != 16_000 {
        return Err(format!("{}: the clip is {} Hz, not 16000", path.display(), wav.rate));
    }
    Ok(wav.samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Paths};
    use crate::control::Stop;
    use crate::plugins::Plugins;
    use crate::settings::SettingsStore;
    use std::net::IpAddr;

    fn ctx(dir: &Path) -> Ctx {
        let hub = Arc::new(Hub::new(1));
        let paths = Paths::new(dir);
        let settings = Arc::new(SettingsStore::new(paths.settings_dir(), hub.clone()));
        settings.register_builtin().unwrap();
        Ctx {
            config: Arc::new(Config::default()),
            paths,
            hub,
            settings,
            stop: Stop::new(),
        }
    }

    fn soon(what: &str, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < end, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn values(items: Json) -> Values {
        serde_json::from_value(items).unwrap()
    }

    #[test]
    fn the_page_is_a_valid_schema_with_the_keys_the_screen_reads() {
        let schema = settings_schema::SettingsSchema::parse(schema_json().to_string().as_bytes()).unwrap();
        let text = serde_json::to_string(&schema).unwrap();
        for key in [
            "status", "microphone", "homeassistant", "last", "others", "wake-word", "sensitivity", "speak", "volume", "room",
            "quiet-hours", "quiet-from", "quiet-until", "pair", "forget",
        ] {
            assert!(text.contains(&format!("\"{key}\"")), "{key}");
        }
        let registered = register();
        let actions: Vec<&str> = registered["actions"].as_array().unwrap().iter().filter_map(|a| a["key"].as_str()).collect();
        assert_eq!(actions, ["pair", "forget"]);
    }

    #[test]
    fn the_choices_are_the_pages_defaults_until_the_owner_says_otherwise() {
        let defaults = Choices::read(&Values::new());
        assert_eq!(defaults.word, "okay_nabu");
        assert_eq!(defaults.sensitivity, Sensitivity::Normal);
        assert!(defaults.speak);
        assert_eq!(defaults.volume, 50);
        assert_eq!(defaults.room, "Living Room");
        assert_eq!(defaults.quiet, Some((1, 7)));

        let set = Choices::read(&values(json!({
            "wake-word": {"type": "choice", "value": "hey_jarvis"},
            "sensitivity": {"type": "choice", "value": "strict"},
            "speak": {"type": "bool", "value": false},
            "volume": {"type": "integer", "value": 80},
            "room": {"type": "text", "value": "  Kitchen "},
            "quiet-hours": {"type": "bool", "value": true},
            "quiet-from": {"type": "integer", "value": 22},
            "quiet-until": {"type": "integer", "value": 6},
        })));
        assert_eq!(set.word, "hey_jarvis");
        assert_eq!(set.sensitivity, Sensitivity::Strict);
        assert!(!set.speak);
        assert_eq!((set.volume, set.room.as_str(), set.quiet), (80, "Kitchen", Some((22, 6))));

        let off = Choices::read(&values(json!({"quiet-hours": {"type": "bool", "value": false},
                                                "wake-word": {"type": "choice", "value": "no_such_word"}})));
        assert_eq!((off.quiet, off.word), (None, "okay_nabu"));
    }

    #[test]
    fn quiet_hours_run_between_the_hours_wrap_midnight_and_a_tv_that_does_not_know_the_hour_is_quiet() {
        let day = Some((9, 17));
        assert!(!is_quiet(day, Some(8)) && is_quiet(day, Some(9)) && is_quiet(day, Some(16)) && !is_quiet(day, Some(17)));
        let night = Some((22, 6));
        assert!(is_quiet(night, Some(22)) && is_quiet(night, Some(23)) && is_quiet(night, Some(0)) && is_quiet(night, Some(5)));
        assert!(!is_quiet(night, Some(6)) && !is_quiet(night, Some(12)) && !is_quiet(night, Some(21)));
        assert!(is_quiet(Some((3, 3)), Some(12)), "the same hour twice is all day");
        assert!(!is_quiet(None, Some(3)) && !is_quiet(None, None), "off is never quiet");
        assert!(is_quiet(day, None));
        // the owner's morning test: 01:00 to 07:00 is quiet at 05:00 and not at 07:00
        assert!(is_quiet(Some((1, 7)), Some(5)) && !is_quiet(Some((1, 7)), Some(7)));
    }

    #[test]
    fn the_facts_say_how_it_stands_and_never_what_was_said() {
        let mut view = View { network: true, phrase: "Okay Nabu".into(), others: "Following the Living Room".into(), ..View::default() };
        assert_eq!(view.status(), "Not paired");
        assert_eq!((view.microphone().as_str(), view.home_assistant().as_str(), view.last().as_str()), ("Closed", "Not paired", "None yet"));
        view.window_min = Some(5);
        assert_eq!(view.status(), "Ready to pair: add the TV in Home Assistant (5 min left)");
        assert_eq!(view.home_assistant(), "Waiting for Home Assistant to add the TV");

        view.paired = true;
        view.window_min = None;
        assert_eq!(view.status(), "Starting");
        view.engine_error = Some("it is busy".into());
        assert_eq!(view.status(), "The microphone did not start: it is busy");
        view.engine_error = None;
        view.engine = Some(Status { capturing: true, ..Status::default() });
        assert_eq!(view.status(), "Paired, waiting for Home Assistant");
        assert_eq!(view.microphone(), "Open, listening for the wake word");
        view.connected = true;
        view.text_only = true;
        assert_eq!(view.status(), "Listening for Okay Nabu, text only");
        assert_eq!(view.home_assistant(), "Connected to Home Assistant");
        view.engine = Some(Status { capturing: true, in_turn: true, ..Status::default() });
        assert_eq!(view.status(), "Talking with Home Assistant");
        view.engine = Some(Status { capturing: true, silent: true, ..Status::default() });
        assert_eq!(view.microphone(), "Open, but it sends no sound");
        view.engine = Some(Status { error: Some("no card".into()), ..Status::default() });
        assert_eq!(view.microphone(), "Not working: no card");

        view.last = Some((Duration::from_secs(200), true));
        assert_eq!(view.last(), "Answered 3 min ago");
        view.last = Some((Duration::from_secs(5), false));
        assert_eq!(view.last(), "Did not work just now");
        view.stopped = Some("LG's audio processor reset".into());
        assert_eq!(view.status(), "Stopped: LG's audio processor reset");
        view.network = false;
        assert_eq!(view.status(), "Waiting for the network");

        let facts = view.facts();
        for key in ["status", "microphone", "homeassistant", "last", "others"] {
            assert!(facts[key].as_str().is_some_and(|s| s.chars().count() <= MAX_FACT_LINE), "{key}");
        }
        assert_eq!(ago(Duration::from_secs(7200)), "2 h ago");
        assert_eq!(ago(Duration::from_secs(3 * 86_400)), "3 d ago");
    }

    #[test]
    fn the_guard_trips_when_the_audio_processors_count_rises_after_the_microphone_opened() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("aud_dsp_reset_count");
        std::fs::write(&file, "0\n").unwrap();
        let mut guard = DspGuard::new(file.clone());
        let now = Instant::now();
        assert!(!guard.tripped(now + Duration::from_secs(60)), "not armed: closed microphone");
        guard.arm();
        assert!(!guard.tripped(now + Duration::from_secs(60)));
        std::fs::write(&file, "1\n").unwrap();
        assert!(!guard.tripped(now + Duration::from_secs(60)), "it looks only every few seconds");
        assert!(guard.tripped(now + Duration::from_secs(120)));
        guard.disarm();
        assert!(!guard.tripped(now + Duration::from_secs(240)));
        // a count that was already above zero when the microphone opened is the baseline
        guard.arm();
        assert!(!guard.tripped(now + Duration::from_secs(600)));
        std::fs::remove_file(&file).unwrap();
        assert!(!guard.tripped(now + Duration::from_secs(700)), "an unreadable count is no reset");
    }

    #[test]
    fn the_environment_keeps_the_key_and_the_last_turn_and_shows_what_it_is_told() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(Hub::new(1));
        let surface = Surface::new(hub.clone());
        surface.set_enabled(true);
        let keys = KeyFile::new(dir.path().join("voice-assistant").join("key"));
        let mut env = TvEnv::new(surface, keys.clone());
        assert!(!env.keyed());
        env.paired([7; 32]);
        assert!(env.keyed() && env.new_key);
        assert_eq!(keys.load(), [7; 32]);

        env.satellite(Effect::Connected);
        assert!(env.connected);
        env.satellite(Effect::Disconnected);
        assert!(!env.connected);
        env.set_volume(0);
        assert_eq!(env.volume, Some(1));

        env.show(&Shown::Listening);
        assert_eq!(hub.get(TOPIC).unwrap()["turn"]["phase"], "listening");
        env.show(&Shown::Reply { text: "Done".into(), ok: true });
        assert!(env.last.is_none(), "the turn ends at idle");
        env.show(&Shown::Idle);
        assert!(env.last.is_some_and(|(_, ok)| ok));
        assert!(!env.command("what time is it").ok);
    }

    fn request(method: &str, peer: &str) -> Request {
        Request {
            method: method.to_owned(),
            path: "/v1/voice-assistant/selftest".to_owned(),
            query: Vec::new(),
            headers: Vec::new(),
            body: Vec::new(),
            peer: peer.parse::<IpAddr>().unwrap(),
        }
    }

    #[test]
    fn the_selfcheck_answers_only_the_tv_itself_by_post_and_only_while_the_microphone_is_open() {
        let status = |r: Response| r.status;
        assert_eq!(status(handle(&request("POST", "192.168.1.10"), "selftest")), 403);
        assert_eq!(status(handle(&request("POST", "127.0.0.1"), "other")), 404);
        assert_eq!(status(handle(&request("GET", "127.0.0.1"), "selftest")), 405);
        assert_eq!(status(handle(&request("POST", "127.0.0.1"), "selftest")), 409, "no microphone is open here");
    }

    #[test]
    fn the_clip_must_be_a_16_khz_wav() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_clip(&dir.path().join("none.wav")).unwrap_err().contains("none.wav"));
        let wav = |rate: u32| {
            let data: Vec<u8> = [0i16, 100, -100, 5].iter().flat_map(|s| s.to_le_bytes()).collect();
            let mut out = b"RIFF".to_vec();
            out.extend((36 + data.len() as u32).to_le_bytes());
            out.extend(b"WAVEfmt ");
            out.extend(16u32.to_le_bytes());
            out.extend([1, 0, 1, 0]);
            out.extend(rate.to_le_bytes());
            out.extend((rate * 2).to_le_bytes());
            out.extend([2, 0, 16, 0]);
            out.extend(b"data");
            out.extend((data.len() as u32).to_le_bytes());
            out.extend(data);
            out
        };
        let good = dir.path().join("good.wav");
        std::fs::write(&good, wav(16_000)).unwrap();
        assert_eq!(read_clip(&good).unwrap(), [0, 100, -100, 5]);
        let slow = dir.path().join("slow.wav");
        std::fs::write(&slow, wav(8000)).unwrap();
        assert!(read_clip(&slow).unwrap_err().contains("8000 Hz"));
    }

    #[test]
    fn switched_on_it_serves_its_page_and_the_screens_topic_and_off_takes_them_away() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let plugins = Plugins::new(&ctx);
        let threads = [plugins.spawn(&ctx).unwrap(), spawn(&ctx, plugins.shared().clone()).unwrap()];
        let enabled = || ctx.hub.get(TOPIC).unwrap_or_default()["enabled"].clone();
        let facts = || ctx.hub.get(&format!("plugin.{ID}.facts")).unwrap_or_default()["facts"].clone();
        let approve = || plugins.shared().approval_of(ID);
        soon("offered, off", || approve().is_some());
        assert_eq!(enabled(), false, "off: the screen is told it is off");

        plugins.switch(ID, true, approve().as_deref()).unwrap();
        soon("on", || enabled() == true);
        soon("its facts", || {
            let status = facts()["status"].clone();
            status == "Not paired" || status == "Waiting for the network"
        });
        assert_eq!(facts()["microphone"], "Closed", "unpaired: the microphone is not opened");
        assert!(!dir.path().join("voice-assistant/audiod.sock").exists());

        plugins.switch(ID, false, None).unwrap();
        soon("off", || enabled() == false);
        ctx.stop.request();
        for t in threads {
            t.join().unwrap();
        }
    }
}
