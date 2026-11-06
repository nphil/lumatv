//! Cameras through Scrypted (`docs/research/scrypted-cameras-design.md`): the TV's cameras come from the LumaShow bridge that runs in Scrypted,
//! not from Home Assistant. This module is the helper's side of it.
//!
//! | Part | What |
//! |---|---|
//! | [`store`], [`link`] | the saved link (`<data>/scrypted/link.json`, 0600) and its command line: `scrypted-link` makes the TV's own token and prints only its hash |
//! | [`tls`], [`http`], [`bridge`] | HTTPS with the owner's pin as the only trust, a small HTTP/1.1 client, the bridge's calls and readers |
//! | [`sse`], [`multipart`], [`events`], [`policy`] | the push channel, the stills stream, the event record, the pop-up rules (a pure function) |
//! | [`stills`] | one stream a camera while the shell asks, the newest picture kept in memory |
//! | [`options`] | the Cameras page of Settings, a first-party plugin, and the owner's choices |
//! | [`routes`] | the loopback API under `/v1/cameras` |
//! | [`crate::live`] | the full view: RTSP from Scrypted (or the bridge's playback) to the GFN feeder and the video plane |
//!
//! **Off until linked.** With no link file nothing connects, nothing is fetched, no thread but the manager's slow look at the file exists and
//! no feeder starts. With a link the TV connects while it is awake (one kept-alive connection for the calls and the push channel) and closes
//! everything in standby. The link is made by the owner on the TV (`scrypted-link`): no route, setting, plugin or key press can write it,
//! and the page can only forget it (Principle 0).
//!
//! The topics (design 3.4, retained): `cameras` (`state`, `reason`, `now_skew_ms`, `rev`, the cameras the TV shows), `cameras.popup` (the card
//! the shell shows now, `null` when none), `cameras.live` (the full view's stream, `live::View`) and `cameras.events` (the last thirty pushes).
//! Nothing in a topic, an answer or a log carries the token, a stream address or the bridge's address.

pub mod bridge;
pub mod events;
pub mod http;
pub mod link;
pub mod multipart;
pub mod options;
pub mod policy;
pub mod popup_stream;
pub mod routes;
pub mod sse;
pub mod stills;
pub mod store;
pub mod tls;

use crate::config::{CamerasConfig, Paths};
use crate::ctx::Ctx;
use crate::hub::Hub;
use crate::images::Images;
use crate::live::{self, Live, Mode};
use crate::plugins::session::Shared;
use crate::screen;
use crate::settings::SettingsStore;
use crate::timeutil::now_ms;
use bridge::{Bridge, Camera, Health, Trouble};
use events::{Event, Ring};
use ha_link::AddressPolicy;
use options::{Identity, Options, Page, PageHooks};
use parking_lot::{Condvar, Mutex};
use policy::{Cooldowns, Decision, Seen};
use serde_json::{Value, json};
use sse::{Sse, SseReader};
use std::collections::{HashMap, VecDeque};
use std::io::{self, BufReader};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use stills::Stills;
use store::Link;

/// How long the manager sleeps while the TV is asleep with a link, if the screen does not change (it looks again then, and sleeps on).
const ASLEEP_WAIT: Duration = Duration::from_secs(30);
/// A camera whose pop-up stream could not be read is tried again after this long.
const LEARN_AGAIN: Duration = Duration::from_secs(600);

/// The retained topics.
pub const TOPIC: &str = "cameras";
pub const POPUP_TOPIC: &str = "cameras.popup";
pub const LIVE_TOPIC: &str = "cameras.live";
pub const EVENTS_TOPIC: &str = "cameras.events";

/// Where the link stands, in the words of design 3.7.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Unlinked,
    Connecting,
    Ready,
    /// The bridge does not answer: the last list is kept.
    Offline,
    /// The bridge refused the token.
    Refused,
    /// The certificate is not the pinned one: nothing was sent.
    Untrusted,
}

impl Status {
    pub fn word(self) -> &'static str {
        match self {
            Self::Unlinked => "unlinked",
            Self::Connecting => "connecting",
            Self::Ready => "ready",
            Self::Offline => "offline",
            Self::Refused => "refused",
            Self::Untrusted => "untrusted",
        }
    }
}

/// How fast the module looks and retries; a test makes it fast.
#[derive(Clone, Debug)]
pub struct Timing {
    /// How often the link file is looked at while there is none.
    pub unlinked_poll: Duration,
    /// How often the file and the screen are looked at while there is a link.
    pub linked_poll: Duration,
    /// How often the camera list and the clock are asked for.
    pub refresh: Duration,
    /// How long to wait before the next try after a failure, in turn (the last repeats).
    pub backoff: Vec<Duration>,
    /// Before trying again after a refusal or an untrusted certificate: only the owner can mend those.
    pub blocked: Duration,
    /// A push channel with nothing, not even its ping, for this long is dead.
    pub silence: Duration,
    /// How long one read of the push channel waits (so that the clock is looked at).
    pub slice: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            unlinked_poll: Duration::from_secs(5),
            linked_poll: Duration::from_secs(1),
            refresh: Duration::from_secs(30),
            backoff: [1, 2, 4, 8, 16, 30].map(Duration::from_secs).to_vec(),
            blocked: Duration::from_secs(30),
            silence: Duration::from_secs(40),
            slice: Duration::from_secs(2),
        }
    }
}

/// Which app SAM has in front (its `getForegroundAppInfo`): a picture in the sub window plays in a pipeline opened for it.
pub type FrontApp = Arc<dyn Fn() -> Option<String> + Send + Sync>;

/// The app in front, from one `getForegroundAppInfo` reply: the application manager's flat shape, or the list of the surface manager's, where the primary
/// entry counts. An empty id and a refused call are none.
fn foreground_of(reply: &Value) -> Option<String> {
    if reply.get("returnValue").and_then(Value::as_bool) == Some(false) {
        return None;
    }
    let entry = match reply.get("foregroundAppInfo").and_then(Value::as_array) {
        Some(list) => list.iter().find(|e| e.get("primary").and_then(Value::as_bool) == Some(true)).or_else(|| list.first()),
        None => Some(reply),
    };
    entry.and_then(|e| e.get("appId")).and_then(Value::as_str).filter(|id| !id.is_empty()).map(str::to_owned)
}

/// What the module acts through; a test names the loopback, its own feeder and dial, and quick timing.
pub struct Deps {
    /// Which addresses the link may name and the token may go to: the home network on the set.
    pub policy: AddressPolicy,
    /// The feeder for the live path; `None` is the config's choice (`cameras.live` with the staged feeder).
    pub feeder: Option<live::FeederSpec>,
    /// How a stream address is dialled; `None` is RTSP over TCP to a home-network address.
    pub dial: Option<live::Dial>,
    /// Which app is in front; `None` is SAM's answer through `luna-send`.
    pub front_app: Option<FrontApp>,
    pub timing: Timing,
}

impl Deps {
    pub fn real() -> Self {
        Self {
            policy: AddressPolicy::HOME,
            feeder: None,
            dial: None,
            front_app: None,
            timing: Timing::default(),
        }
    }
}

/// Deps for tests: the loopback allowed and quick timing.
pub mod testing {
    use super::*;

    pub fn deps() -> Deps {
        Deps {
            policy: AddressPolicy::TESTING,
            feeder: None,
            dial: None,
            front_app: Some(Arc::new(|| Some("org.test.front".to_owned()))),
            timing: Timing {
                unlinked_poll: Duration::from_millis(40),
                linked_poll: Duration::from_millis(40),
                refresh: Duration::from_millis(400),
                backoff: vec![Duration::from_millis(50), Duration::from_millis(100)],
                blocked: Duration::from_millis(300),
                silence: Duration::from_millis(1500),
                slice: Duration::from_millis(100),
            },
        }
    }
}

struct Popup {
    id: u64,
    camera: String,
}

/// Where a recording was when it was paused, so that Play can dial it again there.
#[derive(Clone, Debug)]
pub(crate) struct Held {
    pub camera: String,
    pub mode: Mode,
    pub at: i64,
    pub rate: u8,
    /// The window the shell was granted for the picture (`main`, `sub`), when it is a video window's: Play dials the recording again in the same one.
    pub side: String,
}

struct State {
    /// The link a supervisor is running for (none while unlinked or asleep).
    running: Option<Link>,
    /// There is a link file, and the TV is asleep: only a change of the screen is waited for.
    asleep: bool,
    /// The name and verifier of the link file's token, while there is one (what the page shows for the owner to add this TV to the bridge).
    identity: Option<Identity>,
    status: Status,
    reason: String,
    bridge: Option<Bridge>,
    skew_ms: i64,
    /// Every camera the bridge shares with this TV, and the ones the TV shows (the owner's pick, in its order).
    all: Vec<Camera>,
    shown: Vec<Camera>,
    rev: u64,
    events: Ring,
    popup: Option<Popup>,
    cooldowns: Cooldowns,
    /// The push channel's last sequence number: a reconnect resumes after it.
    last_seq: Option<String>,
    held: Option<Held>,
    /// The window of the stream that plays (`main`, `sub`), when the Cameras full view asked for it on a video window; empty for any other stream.
    side: String,
    /// What each camera's pop-up stream was read to be, where the listing gives no stream a size (`popup_stream::learn`); kept for the helper's life.
    learned: HashMap<String, bridge::Stream>,
    /// The cameras whose stream could not be read, and when: tried again after [`LEARN_AGAIN`].
    unlearned: HashMap<String, Instant>,
    /// A thread reads streams now.
    learning: bool,
}

pub(crate) struct Inner {
    pub(crate) hub: Arc<Hub>,
    pub(crate) settings: Arc<SettingsStore>,
    paths: Paths,
    pub(crate) config: CamerasConfig,
    deps: Deps,
    pub(crate) images: Arc<Images>,
    stop: crate::control::Stop,
    state: Mutex<State>,
    wake: (Mutex<()>, Condvar),
    /// Raised whenever what the supervisor runs for changes: every thread of an earlier one ends.
    generation: AtomicU64,
    pub(crate) live: Live,
    pub(crate) stills: Stills,
    page: Mutex<Option<Arc<Page>>>,
    /// Numbers the calls that start a stream, so that the last of two quick ones is the one that plays.
    pub(crate) ticket: AtomicU64,
    popup_ids: AtomicU64,
    /// Numbers the test events.
    test_ids: AtomicU64,
    /// The pictures of pushed events asked for lately, by event and size.
    pub(crate) thumbs: Mutex<VecDeque<(String, Arc<Vec<u8>>)>>,
}

/// The handle the API and the manager share.
#[derive(Clone)]
pub struct Scrypted {
    pub(crate) inner: Arc<Inner>,
}

impl Scrypted {
    /// Builds the module: the topics exist (unlinked, no popup, idle), the page is registered, and nothing is running. `shared` is the
    /// plugin broker's, where the Cameras page lives.
    pub fn new(ctx: &Ctx, mut deps: Deps, images: Arc<Images>, shared: &Arc<Shared>) -> Self {
        let config = ctx.config.cameras.clone();
        if deps.front_app.is_none() {
            let luna = crate::refreshguard::LunaSource::new(&ctx.config.luna.send);
            deps.front_app = Some(Arc::new(move || {
                let reply = luna.call("luna://com.webos.applicationManager/getForegroundAppInfo", "{}").ok()?;
                foreground_of(&serde_json::from_str(&reply).ok()?)
            }));
        }
        // the feeder: a test's own, else the staged one when the live switch is on
        let feeder = deps.feeder.clone().or_else(|| {
            config.live.then(|| {
                let mut spec = live::default_spec(
                    &ctx.paths.data_dir,
                    &config.feeder_window,
                    &config.feeder_library,
                    Duration::from_secs(u64::from(config.feeder_idle_seconds)),
                );
                if !config.feeder.is_empty() {
                    spec.program = std::path::PathBuf::from(&config.feeder);
                }
                // the window is announced under an id of the brand's (as the saver's player is), never GeForce NOW's
                if let Some(app) = &ctx.config.brand_app_id {
                    spec.extra_args = vec!["-a".to_owned(), format!("{app}.cameras")];
                }
                spec
            })
        });
        let config2 = config.clone();
        let hub = ctx.hub.clone();
        let live = Live::new(
            live::Deps {
                feeder,
                dial: deps.dial.clone().unwrap_or_else(live::real_dial),
            },
            Box::new(move |view| {
                hub.set(LIVE_TOPIC, view);
            }),
        );
        let inner = Arc::new(Inner {
            hub: ctx.hub.clone(),
            settings: ctx.settings.clone(),
            paths: ctx.paths.clone(),
            config,
            deps,
            images,
            stop: ctx.stop.clone(),
            state: Mutex::new(State {
                running: None,
                identity: None,
                asleep: false,
                status: Status::Unlinked,
                reason: unlinked_words().to_owned(),
                bridge: None,
                skew_ms: 0,
                all: Vec::new(),
                shown: Vec::new(),
                rev: 0,
                events: Ring::default(),
                popup: None,
                cooldowns: Cooldowns::default(),
                last_seq: None,
                held: None,
                side: String::new(),
                learned: HashMap::new(),
                unlearned: HashMap::new(),
                learning: false,
            }),
            wake: (Mutex::new(()), Condvar::new()),
            generation: AtomicU64::new(0),
            live,
            stills: Stills::new(
                Duration::from_millis(u64::from(config2.frame_hold_ms)),
                config2.max_picture_bytes as usize,
            ),
            page: Mutex::new(None),
            ticket: AtomicU64::new(0),
            popup_ids: AtomicU64::new(0),
            test_ids: AtomicU64::new(0),
            thumbs: Mutex::new(VecDeque::new()),
        });
        let hooks = {
            let weak = Arc::downgrade(&inner);
            Arc::new(PageHooks {
                all: Mutex::new(Vec::new()),
                linked: Mutex::new(false),
                forget: Box::new(move || {
                    if let Some(inner) = weak.upgrade() {
                        inner.forget();
                    }
                }),
            })
        };
        match Page::start(shared, hooks) {
            Ok(page) => {
                page.say_link(unlinked_words(), None);
                *inner.page.lock() = Some(page);
            }
            Err(why) => crate::log_warn!("cameras: the settings page was not registered: {why}"),
        }
        // the owner's choices change what is shown and how: the manager looks again at once
        {
            let weak = Arc::downgrade(&inner);
            ctx.settings.on_change(options::OWNER, move || {
                if let Some(inner) = weak.upgrade() {
                    inner.options_changed();
                }
            });
        }
        inner.hub.set(POPUP_TOPIC, Value::Null);
        inner.hub.set(EVENTS_TOPIC, json!({"events": []}));
        inner.publish();
        Self { inner }
    }

    /// Starts the manager: it looks at the link file and the screen, and keeps the connection while there is a link and the TV is awake.
    pub fn spawn(&self, ctx: &Ctx) -> io::Result<JoinHandle<()>> {
        let me = self.inner.clone();
        let stop = ctx.stop.clone();
        {
            let me = me.clone();
            ctx.stop.on_stop(move || me.wake());
        }
        // the page's plugin session is spoken to from here: its message handler has big frames in an unoptimised build
        thread::Builder::new()
            .name("scrypted".to_owned())
            .stack_size(http::TLS_STACK)
            .spawn(move || {
                while !stop.is_set() {
                    me.look();
                    me.nap_manager();
                }
                me.teardown(None);
                me.live.stop("the helper stopped");
            })
    }

    /// `GET /v1/cameras`: the topic's value, now.
    pub fn status(&self) -> Value {
        self.inner.view()
    }
}

fn unlinked_words() -> &'static str {
    "Scrypted is not linked: link it from a shell on the TV (lumatv-helper scrypted-link)"
}

impl Inner {
    // ---- the topics ------------------------------------------------------------------------------------------------------------------

    fn camera_json(camera: &Camera) -> Value {
        let native = camera
            .native
            .as_ref()
            .map(|n| json!({"codec": n.codec, "width": n.width, "height": n.height, "fps": n.fps}));
        let prebuffered = camera
            .stream("native")
            .or(camera.native.as_ref())
            .is_some_and(|s| s.prebuffered);
        json!({
            "id": camera.id,
            "name": camera.name,
            "room": camera.room,
            "type": camera.kind,
            "online": camera.online,
            "battery": camera.battery,
            "live": camera.has("live"),
            "stills": camera.has("stills"),
            "recordings": camera.has("recordings"),
            "events": camera.has("events"),
            "clips": camera.has("clips"),
            "since": camera.earliest,
            "native": native,
            "streams": camera
                .streams
                .iter()
                .map(|s| json!({"quality": s.quality, "codec": s.codec, "width": s.width, "height": s.height, "fps": s.fps, "prebuffered": s.prebuffered}))
                .collect::<Vec<_>>(),
            // where this camera's pop-up plays: a window (it has a stream the panel keeps its rate with), the compositor's texture, or stills for a camera
            // on a battery, which is never woken for one
            "popupVideo": if camera.battery { "stills" } else if popup_stream::window_ok(camera) { "window" } else { "texture" },
            "popupStream": camera.learned.as_ref().map(|s| json!({"codec": s.codec, "width": s.width, "height": s.height, "fps": s.fps})),
            "prebuffered": prebuffered,
        })
    }

    fn view_of(state: &State) -> Value {
        json!({
            "state": state.status.word(),
            "reason": state.reason,
            "now_skew_ms": state.skew_ms,
            "rev": state.rev,
            "cameras": state.shown.iter().map(Self::camera_json).collect::<Vec<_>>(),
        })
    }

    pub(crate) fn view(&self) -> Value {
        Self::view_of(&self.state.lock())
    }

    /// Publishes the `cameras` topic (and the line the Settings page shows).
    fn publish(&self) {
        let (view, line, identity) = {
            let state = self.state.lock();
            let line = match state.status {
                Status::Ready => format!("Connected \u{b7} {} cameras", state.shown.len()),
                _ => state.reason.clone(),
            };
            (Self::view_of(&state), line, state.identity.clone())
        };
        self.hub.set(TOPIC, view);
        if let Some(page) = self.page.lock().clone() {
            page.say_link(&line, identity.as_ref());
        }
    }

    pub(crate) fn options(&self) -> Options {
        self.settings
            .effective(options::OWNER)
            .map(|v| Options::read(&v))
            .unwrap_or_default()
    }

    /// The cameras the TV shows: the owner's pick in its order, else every camera in the bridge's.
    fn shown_of(all: &[Camera], show: &[String]) -> Vec<Camera> {
        if show.is_empty() {
            return all.to_vec();
        }
        show.iter()
            .filter_map(|id| all.iter().find(|c| &c.id == id))
            .cloned()
            .collect()
    }

    /// The owner changed the page: what is shown, how fast the grid's pictures come and which events the channel asks for follow.
    fn options_changed(&self) {
        let options = self.options();
        self.stills.set_fps(options::grid_fps(&options.grid));
        self.rebuild_shown();
        self.wake();
    }

    /// Recomputes the shown list from the bridge's and the owner's pick; publishes and replaces the page when it changed.
    fn rebuild_shown(&self) {
        let options = self.options();
        let (changed, names, ids) = {
            let mut state = self.state.lock();
            let mut shown = Self::shown_of(&state.all, &options.show);
            for camera in &mut shown {
                camera.learned = state.learned.get(&camera.id).cloned();
            }
            let before: Vec<Value> = state.shown.iter().map(Self::camera_json).collect();
            let after: Vec<Value> = shown.iter().map(Self::camera_json).collect();
            let changed = before != after;
            if changed {
                state.shown = shown;
                state.rev += 1;
                let ids: Vec<String> = state.shown.iter().map(|c| c.id.clone()).collect();
                // what was heard of a camera that is not shown is forgotten
                state.events.retain(|e| ids.contains(&e.camera));
                if state
                    .popup
                    .as_ref()
                    .is_some_and(|p| !ids.contains(&p.camera))
                {
                    state.popup = None;
                    self.hub.set(POPUP_TOPIC, Value::Null);
                }
                self.hub.set(EVENTS_TOPIC, state.events.json());
            }
            let names: Vec<(String, String)> = state
                .shown
                .iter()
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect();
            let ids: Vec<String> = state.shown.iter().map(|c| c.id.clone()).collect();
            (changed, names, ids)
        };
        if changed {
            self.publish();
            self.stills.keep_only(&ids);
        }
        if let Some(page) = self.page.lock().clone() {
            page.show(&names);
        }
    }

    /// The bridge's camera list, as it is now.
    fn apply_cameras(&self, all: Vec<Camera>) {
        {
            let mut state = self.state.lock();
            if let Some(page) = self.page.lock().as_ref() {
                *page.hooks().all.lock() = all
                    .iter()
                    .map(|c| (c.id.clone(), c.name.clone(), c.room.clone()))
                    .collect();
            }
            state.all = all;
        }
        self.rebuild_shown();
    }

    // ---- the manager ----------------------------------------------------------------------------------------------------------------

    fn wake(&self) {
        let _guard = self.wake.0.lock();
        self.wake.1.notify_all();
    }

    /// Sleeps until something asks for a look or the time for one comes.
    fn nap_manager(&self) {
        let (running, asleep) = {
            let state = self.state.lock();
            (state.running.is_some(), state.asleep)
        };
        if asleep && !self.stop.is_set() {
            // a TV asleep with a link costs nothing: the hub says the moment the screen changes, and nothing else matters until then
            let seq = self.hub.events_after(None, None, None).seq;
            self.hub.wait_events(
                Some(seq),
                Some(self.hub.boot()),
                Some(&["screen".to_owned()]),
                ASLEEP_WAIT,
            );
            return;
        }
        let wait = if running {
            self.deps.timing.linked_poll
        } else {
            self.deps.timing.unlinked_poll
        };
        let mut guard = self.wake.0.lock();
        self.wake.1.wait_for(&mut guard, wait);
    }

    /// Sleeps `wait`, or until the supervisor of generation `gen` is no longer wanted; true while it is.
    fn nap(&self, generation: u64, wait: Duration) -> bool {
        let end = Instant::now() + wait;
        let mut guard = self.wake.0.lock();
        while self.current(generation) && Instant::now() < end {
            self.wake.1.wait_until(
                &mut guard,
                end.min(Instant::now() + Duration::from_millis(200)),
            );
        }
        self.current(generation)
    }

    /// How long to wait before opening the push channel again after it failed.
    fn first_backoff(&self) -> Duration {
        self.deps
            .timing
            .backoff
            .first()
            .copied()
            .unwrap_or(Duration::from_secs(1))
    }

    fn current(&self, generation: u64) -> bool {
        !self.stop.is_set() && self.generation.load(Ordering::SeqCst) == generation
    }

    /// One look: the link file and the screen decide what runs.
    fn look(self: &Arc<Self>) {
        let loaded = store::load(&self.paths.scrypted_link_file(), self.deps.policy);
        let awake = screen::is_on(&self.hub);
        let (want, problem) = match loaded {
            Ok(link) => (link, None),
            Err(why) => (None, Some(why.to_string())),
        };
        let target = if awake { want.clone() } else { None };
        let identity = want.as_ref().map(|link| Identity {
            device: link.device.clone(),
            verifier: link.token.verifier(),
        });
        let (running, identity_changed) = {
            let mut state = self.state.lock();
            state.asleep = want.is_some() && !awake;
            let changed = state.identity != identity;
            state.identity = identity;
            (state.running.clone(), changed)
        };
        if identity_changed {
            self.publish();
        }
        if running != target {
            // what ran for the earlier link or the earlier waking is ended before anything new starts
            if running.is_some() {
                self.teardown(None);
            }
            if let Some(link) = target {
                self.start(link);
            }
        }
        // the states with nothing running say why
        if self.state.lock().running.is_none() {
            match (&want, &problem) {
                (None, None) => self.settle(Status::Unlinked, unlinked_words(), true),
                (None, Some(why)) => self.settle(
                    Status::Unlinked,
                    &format!("The saved link cannot be used: {why}"),
                    true,
                ),
                (Some(_), _) => self.settle(Status::Offline, "The TV is asleep", false),
            }
        }
    }

    /// Sets the status and its words when they are not those already; `clear` forgets the cameras (there is no link).
    fn settle(&self, status: Status, reason: &str, clear: bool) {
        let changed = {
            let mut state = self.state.lock();
            let changed = state.status != status || state.reason != reason;
            state.status = status;
            state.reason = reason.to_owned();
            changed
        };
        if clear {
            self.forget_everything();
        }
        if changed {
            self.publish();
        }
        if let Some(page) = self.page.lock().as_ref() {
            *page.hooks().linked.lock() = status != Status::Unlinked;
        }
    }

    /// No link: no cameras, no events, no card, no stream, no picture.
    fn forget_everything(&self) {
        let had = {
            let mut state = self.state.lock();
            let had = !state.all.is_empty() || !state.events.is_empty() || state.popup.is_some();
            if had {
                state.all.clear();
                state.shown.clear();
                state.rev += 1;
                state.events.clear();
                state.popup = None;
                state.cooldowns.clear();
                state.last_seq = None;
                state.held = None;
            }
            had
        };
        if had {
            self.hub.set(POPUP_TOPIC, Value::Null);
            self.hub.set(EVENTS_TOPIC, json!({"events": []}));
            self.stills.clear();
            if let Some(page) = self.page.lock().clone() {
                *page.hooks().all.lock() = Vec::new();
                page.show(&[]);
            }
            self.publish();
        }
    }

    /// Ends what runs for a link: the supervisor and its threads (by the generation), the stream, the pictures, the connection.
    fn teardown(&self, why: Option<&str>) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.live.stop(why.unwrap_or("the link ended"));
        self.stills.set_bridge(None);
        let bridge = {
            let mut state = self.state.lock();
            state.running = None;
            state.held = None;
            state.bridge.take()
        };
        if let Some(bridge) = bridge {
            bridge.client().close_kept();
        }
        self.wake();
    }

    /// Starts the supervisor for `link`.
    fn start(self: &Arc<Self>, link: Link) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        {
            let mut state = self.state.lock();
            state.running = Some(link.clone());
            state.status = Status::Connecting;
            state.reason = "Connecting to Scrypted".to_owned();
        }
        if let Some(page) = self.page.lock().as_ref() {
            *page.hooks().linked.lock() = true;
        }
        self.publish();
        let me = self.clone();
        let spawned = thread::Builder::new()
            .name("scrypted-link".to_owned())
            .stack_size(http::TLS_STACK)
            .spawn(move || me.supervise(link, generation));
        if let Err(e) = spawned {
            crate::log_warn!("cameras: the link's thread did not start: {e}");
            self.state.lock().running = None;
        }
    }

    /// The owner forgot the link (Settings' button): the file goes and so does everything that was made of it, at once.
    pub(crate) fn forget(&self) {
        store::clear(&self.paths.scrypted_link_file());
        self.state.lock().identity = None;
        self.teardown(Some("the link was forgotten"));
        self.settle(Status::Unlinked, unlinked_words(), true);
        self.wake();
    }

    // ---- the supervisor ---------------------------------------------------------------------------------------------------------------

    /// What a failed call says of the link.
    fn status_of(trouble: &Trouble) -> (Status, String) {
        if trouble.untrusted() {
            (Status::Untrusted, trouble.words())
        } else if trouble.refused() {
            (Status::Refused, trouble.words())
        } else {
            (Status::Offline, trouble.words())
        }
    }

    fn failed(&self, generation: u64, trouble: &Trouble) {
        if !self.current(generation) {
            return;
        }
        let (status, reason) = Self::status_of(trouble);
        crate::log_info!("cameras: link {}", status.word());
        // a link the bridge refuses or whose certificate is not the pinned one has no cameras to show; one that merely does not answer keeps its
        // last list (the tiles dim, with their age)
        self.stills.set_bridge(None);
        self.live.stop(&reason);
        let clear = matches!(status, Status::Refused | Status::Untrusted);
        {
            let mut state = self.state.lock();
            state.bridge = None;
            state.status = status;
            state.reason = reason;
        }
        if clear {
            self.apply_cameras(Vec::new());
        }
        self.publish();
    }

    /// The health and the camera list: the first calls of a connection, which also say whether the token and the pin are good.
    fn connect(&self, bridge: &Bridge) -> Result<(Health, Vec<Camera>, i64), Trouble> {
        let before = now_ms() as i64;
        let health = bridge.health()?;
        let after = now_ms() as i64;
        let cameras = bridge.cameras()?;
        // the bridge's clock against the TV's, at the middle of the call
        let skew = health.now - (before + after) / 2;
        Ok((health, cameras, skew))
    }

    fn supervise(self: &Arc<Self>, link: Link, generation: u64) {
        let dialer = tls::Dialer::new(link.address, link.pin);
        let client = http::Client::new(dialer, link.token.clone());
        let bridge = Bridge::new(client.clone());
        let timing = self.deps.timing.clone();
        let mut tries = 0usize;
        while self.current(generation) {
            match self.connect(&bridge) {
                Ok((health, cameras, skew)) => {
                    tries = 0;
                    if !self.current(generation) {
                        break;
                    }
                    self.ready(&bridge, &health, cameras, skew);
                    self.learn_popup_streams(generation, &bridge);
                    let ended = self.serve(generation, &bridge, &health);
                    if !self.current(generation) {
                        break;
                    }
                    if let Err(trouble) = ended {
                        self.failed(generation, &trouble);
                        let wait = if trouble.refused() || trouble.untrusted() {
                            timing.blocked
                        } else {
                            self.first_backoff()
                        };
                        if !self.nap(generation, wait) {
                            break;
                        }
                    }
                }
                Err(trouble) => {
                    self.failed(generation, &trouble);
                    let wait = if trouble.refused() || trouble.untrusted() {
                        timing.blocked
                    } else {
                        timing
                            .backoff
                            .get(tries)
                            .or(timing.backoff.last())
                            .copied()
                            .unwrap_or(Duration::from_secs(5))
                    };
                    tries += 1;
                    if !self.nap(generation, wait) {
                        break;
                    }
                    if self.current(generation) {
                        let mut state = self.state.lock();
                        if state.status != Status::Ready {
                            state.status = Status::Connecting;
                            state.reason = "Connecting to Scrypted".to_owned();
                        }
                        drop(state);
                        self.publish();
                    }
                }
            }
        }
        client.close_kept();
    }

    /// The link is good: the bridge is there for the routes and the pictures, and the list is the bridge's.
    fn ready(&self, bridge: &Bridge, health: &Health, cameras: Vec<Camera>, skew: i64) {
        let note = if health.has("live") && health.has("stream") {
            String::new()
        } else {
            "The bridge in Scrypted is too old for live video and pop-ups: update it".to_owned()
        };
        {
            let mut state = self.state.lock();
            state.bridge = Some(bridge.clone());
            state.skew_ms = skew;
            state.status = Status::Ready;
            state.reason = note;
        }
        self.stills.set_bridge(Some(bridge.clone()));
        self.stills.set_fps(options::grid_fps(&self.options().grid));
        self.apply_cameras(cameras);
        self.publish();
    }

    /// Reads, on a thread of its own and one camera after another, the pop-up stream of each shown camera whose listing gives no stream a size
    /// (`popup_stream::to_learn`), and publishes what was read. Nothing is read with live pop-ups off; a stream that could not be read waits
    /// [`LEARN_AGAIN`].
    fn learn_popup_streams(self: &Arc<Self>, generation: u64, bridge: &Bridge) {
        if !self.config.popup_live || !self.live.available() {
            return;
        }
        let todo: Vec<String> = {
            let mut state = self.state.lock();
            if state.learning {
                return;
            }
            let todo: Vec<String> = state
                .shown
                .iter()
                .filter(|c| popup_stream::to_learn(c) && state.unlearned.get(&c.id).is_none_or(|at| at.elapsed() >= LEARN_AGAIN))
                .map(|c| c.id.clone())
                .collect();
            if todo.is_empty() {
                return;
            }
            state.learning = true;
            todo
        };
        let (me, bridge) = (self.clone(), bridge.clone());
        let spawned = thread::Builder::new()
            .name("cameras-learn".to_owned())
            .stack_size(http::TLS_STACK)
            .spawn(move || {
                for camera in todo {
                    if !me.current(generation) {
                        break;
                    }
                    let read = popup_stream::learn(|step| bridge.live_within(&camera, "light", step.accept()), |address| me.live.probe(address));
                    let mut state = me.state.lock();
                    match read {
                        Ok(stream) => {
                            crate::log_info!(
                                "cameras: pop-up stream for camera {camera} read from the stream: {} {}x{} at {} a second",
                                stream.codec,
                                stream.width,
                                stream.height,
                                stream.fps
                            );
                            state.unlearned.remove(&camera);
                            state.learned.insert(camera, stream);
                        }
                        Err(why) => {
                            crate::log_info!("cameras: pop-up stream for camera {camera} not read: {why}");
                            state.unlearned.insert(camera, Instant::now());
                        }
                    }
                }
                me.state.lock().learning = false;
                me.rebuild_shown();
            });
        if let Err(e) = spawned {
            crate::log_warn!("cameras: the pop-up stream reader did not start: {e}");
            self.state.lock().learning = false;
        }
    }

    /// Holds the connection: reads the push channel, asks for the camera list and the clock now and then. Returns when the generation is
    /// over (`Ok`) or the bridge went (`Err`, and the supervisor connects again).
    fn serve(
        self: &Arc<Self>,
        generation: u64,
        bridge: &Bridge,
        health: &Health,
    ) -> Result<(), Trouble> {
        let timing = &self.deps.timing;
        let mut next_refresh = Instant::now() + timing.refresh;
        let mut push: Option<Push> = None;
        let mut push_after = Instant::now();
        // a bridge with no push channel (version 1.0) is not asked again for a long while
        let mut push_wanted = health.has("stream") || health.features.is_empty();
        while self.current(generation) {
            let shown: Vec<String> = self
                .state
                .lock()
                .shown
                .iter()
                .map(|c| c.id.clone())
                .collect();
            let motion = self.options().wants_motion(&shown);
            if push.as_ref().is_some_and(|p| p.motion != motion) {
                // the kinds the owner wants changed: the channel is opened again with them
                push = None;
                push_after = Instant::now();
            }
            if push.is_none() && push_wanted && Instant::now() >= push_after {
                let last = self.state.lock().last_seq.clone();
                match bridge.open_events(motion, last.as_deref(), timing.slice) {
                    Ok(body) => {
                        push = Some(Push {
                            reader: SseReader::new(BufReader::new(body)),
                            last_data: Instant::now(),
                            motion,
                        })
                    }
                    Err(trouble)
                        if trouble.refused() || trouble.untrusted() || trouble.unreachable() =>
                    {
                        return Err(trouble);
                    }
                    Err(Trouble::Bridge {
                        status: 404 | 501, ..
                    }) => {
                        push_wanted = false;
                        crate::log_info!(
                            "cameras: the bridge has no push channel; pop-ups are off until it is updated"
                        );
                    }
                    Err(_) => push_after = Instant::now() + self.first_backoff(),
                }
            }
            if let Some(p) = push.as_mut() {
                match p.reader.next_event() {
                    Ok(Some(Sse::Event { id, name, data })) => {
                        p.last_data = Instant::now();
                        self.heard(&name, id.as_deref(), &data);
                    }
                    Ok(Some(Sse::Comment)) => p.last_data = Instant::now(),
                    Ok(Some(Sse::Idle)) => {
                        if p.last_data.elapsed() >= timing.silence {
                            push = None;
                            push_after = Instant::now();
                        }
                    }
                    Ok(None) | Err(_) => {
                        push = None;
                        push_after = Instant::now() + self.first_backoff();
                    }
                }
            } else if !self.nap(generation, Duration::from_millis(100)) {
                return Ok(());
            }
            if Instant::now() >= next_refresh {
                next_refresh = Instant::now() + timing.refresh;
                let (health, cameras, skew) = self.connect(bridge)?;
                let _ = health;
                if self.current(generation) {
                    self.state.lock().skew_ms = skew;
                    self.apply_cameras(cameras);
                    self.publish();
                    self.learn_popup_streams(generation, bridge);
                }
            }
        }
        Ok(())
    }

    /// One thing the push channel said.
    fn heard(self: &Arc<Self>, name: &str, id: Option<&str>, data: &str) {
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return;
        };
        match name {
            "hello" => {
                if let Some(now) = value["now"].as_i64() {
                    self.state.lock().skew_ms = now - now_ms() as i64;
                }
            }
            // what was missed is older than the bridge keeps: the next reconnect starts from now
            "reset" => self.state.lock().last_seq = None,
            "event" => {
                let seq = id.filter(|s| {
                    !s.is_empty() && s.len() <= 18 && s.bytes().all(|b| b.is_ascii_digit())
                });
                if let Some(seq) = seq {
                    self.state.lock().last_seq = Some(seq.to_owned());
                }
                if let Some(event) = Event::parse(&value, seq.and_then(|s| s.parse().ok())) {
                    self.on_event(event, false);
                }
            }
            _ => {}
        }
    }

    // ---- events and the pop-up --------------------------------------------------------------------------------------------------------

    /// An event from the bridge (or a test): into the list, and through the policy to a card.
    pub(crate) fn on_event(self: &Arc<Self>, event: Event, test: bool) -> Option<Decision> {
        let options = self.options();
        let now = now_ms() as i64;
        let mut state = self.state.lock();
        // only the cameras the TV shows are heard
        let name = state
            .shown
            .iter()
            .find(|c| c.id == event.camera)
            .map(|c| c.name.clone())?;
        state.events.push(event.clone());
        self.hub.set(EVENTS_TOPIC, state.events.json());
        let age = if event.at == 0 {
            0
        } else {
            now + state.skew_ms - event.at
        };
        let State { cooldowns, .. } = &mut *state;
        let decision = policy::decide(
            &options.rules,
            &options.notify_for(&event.camera),
            Seen {
                camera: &event.camera,
                kind: &event.kind,
                phase: event.phase,
                age_ms: age,
            },
            now,
            cooldowns,
        );
        match decision {
            Decision::Pop { lasts_ms } => {
                let id = self.popup_ids.fetch_add(1, Ordering::SeqCst) + 1;
                let label = if event.label.is_empty() {
                    event.kind.clone()
                } else {
                    event.label.clone()
                };
                let name = if name.is_empty() { event.name.clone() } else { name };
                // the screensaver's rules (`saver.rs`): while it is up a camera's video may be kept out, shown over it, or woken for
                let route = crate::saver::route_camera(&self.hub, crate::saver::Seen { kind: &event.kind, name: &name, label: &label, event: &event.id });
                if !route.popup {
                    return Some(decision);
                }
                // a camera on a battery is not woken for a pop-up: its card shows stills
                let battery = state.shown.iter().any(|c| c.id == event.camera && c.battery);
                let live_ok = self.config.popup_live && self.live.available() && !battery;
                // a window's decoder re-times the panel to its stream's rate: a camera with no calm stream has its pop-up drawn as a texture, never in a window
                let texture_only = state.shown.iter().find(|c| c.id == event.camera).is_none_or(|c| !popup_stream::window_ok(c));
                let value = json!({
                    "id": id,
                    "event": event.id,
                    "camera": event.camera,
                    "name": name,
                    "kind": event.kind,
                    "label": label,
                    "at": event.at,
                    "untilMs": now + lasts_ms as i64,
                    "seq": event.seq,
                    "thumbnail": event.thumbnail,
                    "shownAt": now,
                    "overApps": options.rules.over_apps,
                    "overSaver": route.over_saver,
                    "test": test,
                    "source": "scrypted",
                    "live": live_ok,
                    "textureOnly": texture_only,
                });
                crate::log_info!(
                    "cameras: pop-up {id} for {} {} on {name} (camera {}, event {}, source scrypted, {} s{}{})",
                    event.kind,
                    if label == event.kind { String::new() } else { format!("({label})") },
                    event.camera,
                    event.id,
                    lasts_ms / 1000,
                    if test { ", a test" } else { "" },
                    match (live_ok, texture_only) {
                        (false, _) if battery => ", stills only (a camera on a battery)",
                        (false, _) => ", stills only",
                        (true, false) => ", live video where a window is free",
                        (true, true) => ", live video as a texture (no stream the panel keeps its rate with)",
                    },
                );
                state.popup = Some(Popup {
                    id,
                    camera: event.camera.clone(),
                });
                self.hub.set(POPUP_TOPIC, value);
                drop(state);
                self.expire_popup(id, Duration::from_millis(lasts_ms));
            }
            Decision::Update => {
                // the words of the card that is up change, and its time does not
                if let Some(popup) = state.popup.as_ref().filter(|p| p.camera == event.camera)
                    && let Some(mut value) = self.hub.get(POPUP_TOPIC)
                    && let Some(object) = value.as_object_mut()
                {
                    let label = if event.label.is_empty() {
                        event.kind.clone()
                    } else {
                        event.label.clone()
                    };
                    object.insert("event".into(), json!(event.id));
                    object.insert("kind".into(), json!(event.kind));
                    object.insert("label".into(), json!(label));
                    object.insert("seq".into(), json!(event.seq));
                    object.insert("thumbnail".into(), json!(event.thumbnail));
                    let _ = popup;
                    self.hub.set(POPUP_TOPIC, value);
                }
            }
            Decision::Held(why) => {
                crate::log_info!(
                    "cameras: {} on {name} (camera {}, event {}) makes no pop-up: {}",
                    event.kind,
                    event.camera,
                    event.id,
                    match why {
                        policy::Held::Off => "pop-ups are off",
                        policy::Held::Kind => "its kind is not one this camera notifies for",
                        policy::Held::End => "the object left",
                        policy::Held::Cooldown => "the same kind popped up a moment ago",
                        policy::Held::Old => "it happened too long ago",
                    }
                );
            }
        }
        Some(decision)
    }

    /// The card goes by itself when its time is up.
    fn expire_popup(self: &Arc<Self>, id: u64, lasts: Duration) {
        let me = self.clone();
        let _ = thread::Builder::new()
            .name("scrypted-popup".to_owned())
            .stack_size(64 * 1024)
            .spawn(move || {
                if !me.stop.sleep(lasts) {
                    me.clear_popup_if(Some(id));
                }
            });
    }

    /// `POST /v1/cameras/popup/clear` (with `None`), and the card's time being up (with its id).
    pub(crate) fn clear_popup_if(&self, id: Option<u64>) {
        let mut state = self.state.lock();
        if state
            .popup
            .as_ref()
            .is_some_and(|p| id.is_none_or(|id| p.id == id))
        {
            state.popup = None;
            self.hub.set(POPUP_TOPIC, Value::Null);
        }
    }

    // ---- what the routes need ----------------------------------------------------------------------------------------------------------

    /// The bridge, while the link is good.
    pub(crate) fn bridge(&self) -> Result<Bridge, (Status, String)> {
        let state = self.state.lock();
        match (&state.bridge, state.status) {
            (Some(bridge), Status::Ready) => Ok(bridge.clone()),
            (_, status) => Err((status, state.reason.clone())),
        }
    }

    /// A camera the TV shows.
    pub(crate) fn camera(&self, id: &str) -> Option<Camera> {
        self.state.lock().shown.iter().find(|c| c.id == id).cloned()
    }

    pub(crate) fn skew_ms(&self) -> i64 {
        self.state.lock().skew_ms
    }

    pub(crate) fn held(&self) -> Option<Held> {
        self.state.lock().held.clone()
    }

    pub(crate) fn set_held(&self, held: Option<Held>) {
        self.state.lock().held = held;
    }

    pub(crate) fn popup_seq(&self) -> u64 {
        self.test_ids.fetch_add(1, Ordering::SeqCst) + 1
    }
}

/// The push channel while it is open.
struct Push {
    reader: SseReader<BufReader<http::BodyReader>>,
    last_data: Instant,
    motion: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_front_app_is_read_from_either_reply_shape_and_a_refusal_or_an_empty_id_is_none() {
        let flat: Value = serde_json::from_str(include_str!("../../tests/fixtures/luna/getForegroundAppInfo-appmanager.json")).unwrap();
        assert_eq!(foreground_of(&flat).as_deref(), Some("netflix"));
        let list: Value = serde_json::from_str(include_str!("../../tests/fixtures/luna/getForegroundAppInfo.json")).unwrap();
        assert_eq!(foreground_of(&list).as_deref(), Some("com.webos.app.home"));
        assert_eq!(foreground_of(&json!({"returnValue": true, "appId": ""})), None);
        assert_eq!(foreground_of(&json!({"returnValue": false, "errorText": "Denied"})), None);
    }
}
