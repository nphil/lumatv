//! The feeder launches this 64-bit card window; engine decisions stay in the model and media stays out of this loop.
use lumagame_core::{catalog::ArtKind, engine::Codec, ids::GameId, ipc::{Display, Request}};
use lumagame_meta::{Artwork, Cache, CacheLimits, Client, Http, Job, Kind, Loader, Metadata, secrets::Secrets};
use lumatv_gfn_pad::{MAX_PADS, PadConfig, PadEvent, PadRuntime, PadSink, PadState, RawReport, button, menu::{MenuKey, MenuKeys, STICK_RELEASE, strength}};
use lumatv_render::{Demand, now_ns, host::{Engine, HostConfig}};
use lumatv_rustui::{card::{FULLSCREEN, MINIMIZED}, game::{helper::{ControllerClient, ControllerRequest, ControllerReply, Lease}, host::{self, HostEvent, Supervisor}, model::{Command, GameDetails, Input, Key, Model, settings::PadMode}, screens::GameUi, store::{self, Store}}, lifecycle::{Channel, FromApp, ToApp}, window::{self, Event, Spec}};
use lumatv_rustui::game::helper::{OverlayClient, Sample};
use lumatv_rustui::game::diagnostics::{Diagnostics, Event as DiagnosticEvent, Level};
use lumatv_rustui::game::input::Face;
use lumatv_rustui::game::{uplink, video};
use lumatv_rustui::hook::{self, Command as HookCommand, Hook};
use lumatv_rustui::game::model::game::media::VideoRect;
use lumatv_rustui::game::observer::{Observer, engine_word, phase_word};
use std::{collections::VecDeque, io, net::SocketAddr, os::fd::AsRawFd, os::unix::net::UnixDatagram, path::{Path, PathBuf}, sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering}, mpsc::{self, Receiver, SyncSender}}, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};
use ui::{SceneOptions, Ui};

const W: u32 = 1920;
const H: u32 = 1080;
/// How soon the page clears after the player grows to full screen, and comes back after it returns to the card.
const VIDEO_FLAG_MS: u64 = 100;
static END: AtomicBool = AtomicBool::new(false);
static SWAP_SINCE_MS: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TraceState {
    page: lumatv_rustui::game::model::Page,
    foreground: bool,
    phase: &'static str,
    visible: bool,
}

impl TraceState {
    fn of(model: &Model) -> Self {
        Self { page: model.view.page, foreground: model.foreground(), phase: phase_word(model.session().map(|s| s.phase())), visible: model.wants_frames() }
    }
}
fn page_name(page: lumatv_rustui::game::model::Page) -> &'static str {
    use lumatv_rustui::game::model::Page;
    match page {
        Page::Home => "home", Page::Library => "library", Page::Game => "hub",
        Page::Settings => "settings", Page::Launch => "launch",
        Page::Running => "running",
    }
}

#[derive(Default)]
struct LifecycleTrace {
    last: Option<TraceState>,
    sequence: u64,
    pending: Option<u64>,
    decision_logged: bool,
}

impl LifecycleTrace {
    fn enabled(value: Option<&std::ffi::OsStr>) -> Option<Self> {
        (value == Some(std::ffi::OsStr::new("1"))).then(Self::default)
    }

    fn observe(&mut self, state: TraceState, events: u8) -> Option<u64> {
        if events == 0 && self.last == Some(state) { return None; }
        self.last = Some(state);
        self.sequence += 1;
        if events & 3 != 0 {
            self.pending = Some(self.sequence);
            self.decision_logged = false;
        }
        Some(self.sequence)
    }

    fn decision(&mut self) -> Option<u64> {
        if self.decision_logged { return None; }
        let sequence = self.pending?;
        self.decision_logged = true;
        Some(sequence)
    }

    fn swapped(&mut self) -> Option<u64> { self.pending.take() }
}

struct Options {
    app_id: String, assets: PathBuf, shm: PathBuf, engines: PathBuf, runner: Option<PathBuf>, data: PathBuf,
    feeder: PathBuf, window_id: PathBuf, libmali: Option<String>, channel_fd: Option<i32>, hidden: bool, helper: SocketAddr,
}

fn options() -> Result<Options, String> {
    let home = std::env::current_exe().ok().and_then(|p| p.parent().and_then(Path::parent).map(Path::to_owned)).unwrap_or_else(|| PathBuf::from("."));
    let runtime = PathBuf::from("/tmp/org.nphil.lumagame");
    let mut o = Options { app_id: store::APP_ID.into(), assets: home.join("assets"), shm: runtime.join("shm"), engines: home.join("engines"), runner: None,
        data: PathBuf::from("/var/lib/lumagame"), feeder: runtime.join("feed.sock"), window_id: runtime.join("window-id"), libmali: std::env::var("LUMATV_GFX_LIB").ok(), channel_fd: None, hidden: false,
        helper: SocketAddr::from(([127, 0, 0, 1], 18731)) };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--app-id" => o.app_id = value()?, "--assets" => o.assets = value()?.into(), "--shm" => o.shm = value()?.into(),
            "--engines" => o.engines = value()?.into(), "--engine-runner" => o.runner = Some(value()?.into()), "--data" => o.data = value()?.into(),
            "--feeder-socket" => o.feeder = value()?.into(), "--window-id-file" => o.window_id = value()?.into(), "--libmali" => o.libmali = Some(value()?),
            "--channel-fd" => o.channel_fd = Some(value()?.parse::<i32>().map_err(|e| e.to_string())?), "--hidden" => o.hidden = true,
            "--helper" => o.helper = value()?.parse::<SocketAddr>().map_err(|e| e.to_string())?,
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    if !o.helper.ip().is_loopback() { return Err("--helper must be a loopback address".into()); }
    if o.channel_fd.is_some_and(|fd| fd < 3) { return Err("--channel-fd must be an inherited descriptor, not stdin/stdout/stderr".into()); }
    Ok(o)
}

extern "C" fn on_term(_: libc::c_int) { END.store(true, Ordering::Relaxed); }

fn main() {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--diagnostic-send")) {
        std::process::exit(lumatv_rustui::game::diagnostics::sender_main());
    }
    let options = match options() { Ok(o) => o, Err(e) => { eprintln!("lumagame: {e}"); std::process::exit(64); } };
    unsafe { for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] { libc::signal(signal, on_term as extern "C" fn(libc::c_int) as libc::sighandler_t); } }
    let shm = options.shm.clone();
    let diagnostics = Diagnostics::default();
    let started = Instant::now();
    let result = run(options, &diagnostics);
    if result.is_err() { let _ = diagnostics.flush_fatal(started.elapsed().as_millis() as u64, Duration::from_secs(2)); }
    let _ = std::fs::remove_dir_all(shm);
    if let Err(e) = result { eprintln!("lumagame: {e}"); std::process::exit(1); }
}

/// `held` is the direction the pad has sent and not yet released: the model repeats a direction while it is down.
#[derive(Default)]
struct PadKeys { menu: MenuKeys, buttons: u16, held: Option<Key>, pushed: u8 }
#[derive(Default)]
struct LabBuffer { reports: VecDeque<(u8, RawReport)>, names: [Option<String>; MAX_PADS] }
struct MenuSink { active: AtomicBool, mode: AtomicU8, connected: AtomicU32, keys: Mutex<[PadKeys; MAX_PADS]>, lab: Mutex<LabBuffer>, send: UnixDatagram }
struct MenuPads { runtime: PadRuntime, sink: Arc<MenuSink>, receive: UnixDatagram, announced: Mutex<[Option<String>; MAX_PADS]> }

/// The direction a held key stands for, to read its push from the pad's state.
fn direction(key: Key) -> MenuKey {
    match key {
        Key::Up => MenuKey::Up,
        Key::Down => MenuKey::Down,
        Key::Left => MenuKey::Left,
        _ => MenuKey::Right,
    }
}

const KEYS: [Key; 10] = [Key::Up, Key::Down, Key::Left, Key::Right, Key::Ok, Key::Back, Key::Guide, Key::Menu, Key::Prev, Key::Next];

/// Whether the pad still holds `key`'s direction: its d-pad button, or the left stick not yet back under the menu's lower threshold.
fn holds(key: Key, state: &PadState) -> bool {
    match key {
        Key::Up => state.buttons & button::DPAD_UP != 0 || state.left_y > STICK_RELEASE,
        Key::Down => state.buttons & button::DPAD_DOWN != 0 || state.left_y < -STICK_RELEASE,
        Key::Left => state.buttons & button::DPAD_LEFT != 0 || state.left_x < -STICK_RELEASE,
        Key::Right => state.buttons & button::DPAD_RIGHT != 0 || state.left_x > STICK_RELEASE,
        _ => false,
    }
}

impl MenuSink {
    /// One key datagram: the key's index, whether it went down, and the slot of the pad that pressed it.
    fn send(&self, slot: u8, key: Key, down: bool) {
        if let Some(index) = KEYS.iter().position(|k| *k == key) { let _ = self.send.send(&[index as u8, u8::from(down), slot, 0]); }
    }
    /// How hard a held direction is pushed (the fourth byte, sent as a third state of "down"), for the model's repeat to step by.
    fn push(&self, slot: u8, key: Key, strength: u8) {
        if let Some(index) = KEYS.iter().position(|k| *k == key) { let _ = self.send.send(&[index as u8, 2, slot, strength]); }
    }
}
impl PadSink for MenuSink {
    fn pad_event(&self, _: u16, event: PadEvent) {
        let mut keys = self.keys.lock().unwrap_or_else(|e| e.into_inner());
        let active = self.active.load(Ordering::Acquire);
        let mode = self.mode.load(Ordering::Acquire);
        let navigating = active && mode != PadMode::Captured as u8;
        match event {
            PadEvent::Report { slot, mut state, .. } => {
                if let Some(keys) = keys.get_mut(usize::from(slot)) {
                    // Stick movement belongs to the lab; held buttons still track their real release across mode changes.
                    if mode != PadMode::Menu as u8 { state.left_x = 0; state.left_y = 0; state.right_x = 0; state.right_y = 0; }
                    keys.menu.feed(&state, |menu_key| if navigating {
                        let key = Key::from_menu(menu_key);
                        if matches!(key, Key::Up | Key::Down | Key::Left | Key::Right) {
                            if let Some(old) = keys.held.replace(key).filter(|old| *old != key) { self.send(slot, old, false); }
                            keys.pushed = strength(&state, menu_key);
                            self.send(slot, key, true);
                            self.push(slot, key, keys.pushed);
                        } else {
                            self.send(slot, key, true);
                        }
                    });
                    if let Some(key) = keys.held.filter(|key| !holds(*key, &state)) {
                        keys.held = None;
                        if navigating { self.send(slot, key, false); }
                    } else if let Some(key) = keys.held.filter(|_| navigating) {
                        // A stick that is pushed further or eased off changes the repeat's pace; a nudge of 6 % is not worth a message.
                        let now = strength(&state, direction(key));
                        if now.abs_diff(keys.pushed) >= 16 {
                            keys.pushed = now;
                            self.push(slot, key, now);
                        }
                    }
                    if navigating {
                        // The menus act on release, so every face button that sends a key on press owes one on release.
                        for face in Face::ALL {
                            if keys.buttons & face.button() != 0 && state.buttons & face.button() == 0 { self.send(slot, face.key(), false); }
                        }
                    }
                    keys.buttons = state.buttons;
                }
            }
            PadEvent::Connected { slot, name, .. } => {
                self.connected.fetch_or(1 << (u32::from(slot) & 31), Ordering::AcqRel);
                let _ = self.send.send(&[0]);
                if let Some(keys) = keys.get_mut(usize::from(slot)) { *keys = PadKeys::default(); }
                if let Some(known) = self.lab.lock().unwrap_or_else(|e| e.into_inner()).names.get_mut(usize::from(slot)) { *known = Some(name); }
            }
            PadEvent::Disconnected { slot } => {
                self.connected.fetch_and(!(1 << (u32::from(slot) & 31)), Ordering::AcqRel);
                let _ = self.send.send(&[0]);
                if let Some(keys) = keys.get_mut(usize::from(slot)) {
                    if let Some(key) = keys.held { self.send(slot, key, false); }
                    *keys = PadKeys::default();
                }
                let mut lab = self.lab.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(known) = lab.names.get_mut(usize::from(slot)) { *known = None; }
                lab.reports.retain(|(i, _)| *i != slot);
            }
            PadEvent::Guide { slot, pressed } if navigating => self.send(slot, Key::Guide, pressed),
            _ => {}
        }
    }
    fn raw(&self, slot: u8, report: &RawReport) {
        if !self.active.load(Ordering::Acquire) || self.mode.load(Ordering::Acquire) == PadMode::Menu as u8 { return; }
        let mut lab = self.lab.lock().unwrap_or_else(|e| e.into_inner());
        let wake = lab.reports.is_empty();
        if lab.reports.len() == 4096 { lab.reports.pop_front(); }
        lab.reports.push_back((slot, *report));
        if wake { let _ = self.send.send(&[0]); }
    }
}
impl MenuPads {
    fn new() -> io::Result<Self> {
        let (send, receive) = UnixDatagram::pair()?;
        send.set_nonblocking(true)?; receive.set_nonblocking(true)?;
        let sink = Arc::new(MenuSink { active: AtomicBool::new(false), mode: AtomicU8::new(PadMode::Menu as u8), connected: AtomicU32::new(0), keys: Mutex::new(std::array::from_fn(|_| PadKeys::default())), lab: Mutex::new(LabBuffer::default()), send });
        let runtime = PadRuntime::start(PadConfig { boost: None, latency_dir: None, raw: true, ..PadConfig::from_env() }, sink.clone());
        Ok(Self { runtime, sink, receive, announced: Mutex::new(Default::default()) })
    }
    fn sync(&self, wanted: bool) {
        if self.sink.active.swap(wanted, Ordering::AcqRel) != wanted { self.runtime.set_grab(wanted); }
    }
    fn set_mode(&self, mode: PadMode) {
        if self.sink.mode.swap(mode as u8, Ordering::AcqRel) != mode as u8 {
            self.sink.lab.lock().unwrap_or_else(|e| e.into_inner()).reports.clear();
        }
    }
    fn poll(&self, inputs: &mut Vec<Input>, frame: u64) {
        let mut bytes = [0; 4];
        let names = self.sink.lab.lock().unwrap_or_else(|e| e.into_inner()).names.clone();
        if std::mem::replace(&mut *self.announced.lock().unwrap_or_else(|e| e.into_inner()), names.clone()) != names { inputs.push(Input::Controllers(names)); }
        while let Ok(n) = self.receive.recv(&mut bytes) {
            if n == 4 && self.sink.active.load(Ordering::Acquire) {
                if let Some(key) = KEYS.get(usize::from(bytes[0])) {
                    inputs.push(if bytes[1] == 2 { Input::PadPush { key: *key, strength: bytes[3] } } else { Input::PadKey { slot: bytes[2], key: *key, down: bytes[1] != 0 } });
                }
            }
        }
        if self.sink.active.load(Ordering::Acquire) && self.sink.mode.load(Ordering::Acquire) != PadMode::Menu as u8 {
            let mut lab = self.sink.lab.lock().unwrap_or_else(|e| e.into_inner());
            inputs.push(Input::Lab { now_ns: now_ns(), reports: lab.reports.drain(..).collect(), names: lab.names.clone(), frame });
        }
    }
}
impl Drop for MenuPads { fn drop(&mut self) { self.runtime.stop(); } }
struct Details {
    jobs: SyncSender<(GameId, Vec<lumagame_core::ids::ExternalId>, String)>,
    results: Receiver<(GameId, Option<lumagame_meta::Hub>, Option<Metadata>)>,
}
impl Details {
    /// The page-data worker. The public sources need no key; IGDB is added when the owner has configured its keys.
    fn new(secrets: Secrets, dir: PathBuf) -> io::Result<Option<Self>> {
        let igdb = !secrets.twitch_client_id.is_empty() && !secrets.twitch_client_secret.is_empty();
        let client = Client::<Http>::new(secrets);
        let cache = Mutex::new(Cache::open(dir, CacheLimits { bytes: 16 << 20, files: 2048, image_bytes: 1 << 20 }).ok());
        let pace = lumagame_meta::hub::Pacing::new(Duration::from_millis(1000));
        let (jobs, incoming) = mpsc::sync_channel::<(GameId, Vec<lumagame_core::ids::ExternalId>, String)>(4);
        let (finished, results) = mpsc::sync_channel(4);
        std::thread::Builder::new().name("game-details".into()).spawn(move || {
            while let Ok((id, external, title)) = incoming.recv() {
                let steam = external.iter().find(|e| e.ns() == lumagame_core::ids::ExternalId::STEAM).and_then(|e| e.id().parse::<u64>().ok());
                let hub = lumagame_meta::hub::load(&client, &pace, &cache, &lumagame_meta::hub::Lookup { steam, title: &title }, unix_seconds());
                let metadata = if igdb { client.igdb(&external).ok().flatten() } else { None };
                if finished.send((id, hub, metadata)).is_err() { break; }
            }
        })?;
        Ok(Some(Self { jobs, results }))
    }
}

/// What the owner's keyed provider adds to what the public sources gave: it fills gaps and never replaces.
fn game_details(hub: Option<lumagame_meta::Hub>, metadata: Option<Metadata>) -> Option<GameDetails> {
    if hub.is_none() && metadata.is_none() { return None; }
    let mut details = hub.map(GameDetails::from_hub).unwrap_or_default();
    let Some(metadata) = metadata else { return Some(details) };
    let year = metadata.release_date.and_then(|seconds| {
        let seconds: libc::time_t = seconds;
        let mut date = std::mem::MaybeUninit::<libc::tm>::uninit();
        if unsafe { libc::gmtime_r(&seconds, date.as_mut_ptr()) }.is_null() { None }
        else { u32::try_from(unsafe { date.assume_init() }.tm_year + 1900).ok() }
    });
    if details.description.is_empty() { details.description = metadata.summary; }
    details.year = details.year.or(year);
    if details.genres.is_empty() { details.genres = metadata.genres; }
    if details.developer.is_none() && !metadata.developers.is_empty() { details.developer = Some(metadata.developers.join(", ")); }
    Some(details)
}

struct Shell {
    model: Model, host: Supervisor, store: Store, channel: Option<Channel>, lease: Lease, controllers: ControllerClient, art: Loader,
    art_waiting: VecDeque<String>, details: Option<Details>, details_waiting: VecDeque<GameId>, state_saved: bool, media_seq: u64,
    stats: OverlayClient, frames_on: bool,
    diagnostics: Diagnostics,
    observer: Observer, events: Vec<DiagnosticEvent>,
    /// The focused still's card as a player, from the last frame drawn: where a video starts.
    card: Option<VideoRect>,
    /// The frame app is playing a video over LumaGame; it holds the keys until Back closes it.
    video_up: bool,
    /// The file the frame app keeps while its player fills the screen (it has no other way to tell LumaGame); in the app's data directory.
    video_full_flag: PathBuf,
}
impl Shell {
    /// Hands what the observer found to the diagnostics queue, which keeps only what the level asks for.
    fn report(&mut self) { for event in self.events.drain(..) { self.diagnostics.emit(event); } }
    fn observe_level(&mut self, now_ms: u64) {
        self.observer.configure(self.diagnostics.level(), now_ms, &mut self.events);
        self.report();
    }
    /// The model's step, with the session as it then stands handed to diagnostics.
    fn step(&mut self, input: Input, now_ms: u64) -> Vec<Command> {
        let commands = self.model.step(input);
        if self.diagnostics.enabled() {
            self.observer.session(now_ms, self.model.observed(), &mut self.events);
            self.report();
        }
        commands
    }
    fn inputs(&mut self, inputs: impl IntoIterator<Item = Input>, pads: &MenuPads, now_ms: u64) -> io::Result<bool> {
        let mut commands = VecDeque::new();
        self.observe_level(now_ms);
        for input in inputs {
            if self.diagnostics.enabled() {
                match &input {
                    Input::Engine { engine, message } => self.observer.engine_event(now_ms, engine.as_str(), &message.event, &mut self.events),
                    Input::Frames(frames) => self.observer.frames(now_ms, frames, &mut self.events),
                    Input::Display(display) => self.observer.display(now_ms, display, &mut self.events),
                    Input::EngineGone(_) => self.events.push(DiagnosticEvent::Error { ms: now_ms, kind: "engine_gone" }),
                    _ => {}
                }
                self.report();
            }
            if self.video_up && matches!(input, Input::Foreground(false) | Input::Close) {
                // LumaGame is leaving: the player goes with it rather than stay over whatever comes next.
                self.video_up = false;
                if let Err(e) = self.launch_app(video::FRAME_APP, video::close()) { eprintln!("lumagame: closing the player: {e}"); }
            }
            commands.extend(self.step(input, now_ms));
        }
        if self.diagnostics.enabled() {
            self.observer.pads(now_ms, pads.sink.connected.load(Ordering::Acquire), &mut self.events);
            self.observer.tick(now_ms, &mut self.events);
            self.report();
        }
        // Release the pads before an engine receives Launch, including a native app taking the foreground.
        pads.sync(self.model.wants_pads());
        pads.set_mode(self.model.pad_mode());
        while let Some(command) = commands.pop_front() {
            let result = match command {
                Command::Send { engine, request } => {
                    if let Err(e) = self.host.send(&engine, request) {
                        commands.extend(self.step(Input::EngineGone(engine), now_ms));
                        Err(e)
                    } else { Ok(()) }
                }
                Command::SteamOverlay(None) => {
                    self.events.push(DiagnosticEvent::SteamOverlay { ms: now_ms, engine: "other", result: "no_session" });
                    self.report();
                    Ok(())
                }
                Command::SteamOverlay(Some((engine, session))) => {
                    let sent = self.host.send(&engine, Request::SteamOverlay { session });
                    let result = if sent.is_ok() { "sent" } else { "refused" };
                    self.events.push(DiagnosticEvent::SteamOverlay { ms: now_ms, engine: engine_word(engine.as_str()), result });
                    self.report();
                    if sent.is_err() { commands.extend(self.step(Input::EngineGone(engine), now_ms)); }
                    sent
                }
                Command::SaveClip(None) => Ok(()),
                Command::SaveClip(Some((engine, session))) => {
                    let sent = self.host.send(&engine, Request::SaveClip { session });
                    if sent.is_err() { commands.extend(self.step(Input::EngineGone(engine), now_ms)); }
                    sent
                }
                Command::Enable { .. } => { self.host.sync(self.model.state(), self.model.foreground(), self.model.session_active(), now_ms); Ok(()) }
                Command::KillEngine(id) => { self.host.kill(&id, now_ms); Ok(()) }
                Command::SaveState => {
                    let result = self.store.save_state(self.model.state()); self.state_saved = result.is_ok(); result
                }
                Command::SaveCatalog => self.store.save_catalog(self.model.listing()),
                Command::Checkpoint(open) => self.store.checkpoint(&open),
                Command::ClearCheckpoint => if self.state_saved { self.store.clear_checkpoint() } else { Ok(()) },
                Command::LowLatency(on) => { self.lease.set(on, now_ms); Ok(()) }
                Command::FetchArt { url } => { self.art_waiting.push_back(url); Ok(()) }
                Command::FetchDetails { game } => { self.details_waiting.push_back(game); Ok(()) }
                Command::PlayMedia { url } => {
                    if !url.starts_with("https://") { Err(io::Error::new(io::ErrorKind::InvalidInput, "media must use HTTPS")) }
                    else { self.launch_app("com.webos.app.browser", serde_json::json!({"target":url})) }
                }
                // The frame app plays it on the main hardware video window over the still's card (`rect` when no card was drawn).
                Command::PlayVideo { kind, source, rect } => {
                    let _ = std::fs::remove_file(&self.video_full_flag);
                    let launched = self.launch_app(video::FRAME_APP, video::launch(kind, &source, self.card.unwrap_or(rect), &self.video_full_flag));
                    self.video_up = launched.is_ok();
                    launched
                }
                Command::OpenControllerSettings => self.launch_app("com.palm.app.settings", serde_json::json!({})),
                Command::Rumble { slot, low, high } => pads.runtime.rumble(slot, low, high, 500),
                Command::GameSir(change) => {
                    if let Err(error) = self.controllers.request(ControllerRequest::GameSir(change)) { commands.extend(self.model.step(Input::GameSir(Err(error)))); }
                    Ok(())
                }
                Command::Leadjoy { view, change } => {
                    if let Err(error) = self.controllers.request(ControllerRequest::Leadjoy { view, change }) { commands.extend(self.model.step(Input::Leadjoy(Err(error)))); }
                    Ok(())
                }
                Command::Diagnostics(config) => {
                    self.diagnostics.configure(config.clone());
                    self.observe_level(now_ms);
                    self.store.save_diagnostics(&config)
                }
                Command::DiagnosticTest => { self.diagnostics.emit(DiagnosticEvent::Test); Ok(()) }
                Command::Exit => {
                    if let Some(ch) = &mut self.channel { ch.send(&FromApp::Exit("LumaGame closed".into())); }
                    return Ok(true);
                }
            };
            if let Err(e) = result { commands.extend(self.model.step(Input::Notice(e.to_string()))); }
        }
        self.host.sync(self.model.state(), self.model.foreground(), self.model.session_active(), now_ms);
        for command in self.host.lifecycle_commands() {
            let result = self.channel.as_mut().ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "the TV application service is unavailable")).and_then(|ch| ch.try_send(&command));
            if let Err(e) = result { self.host.channel_failed(e.to_string()); }
        }
        self.lease.tick(now_ms);
        let visible = self.model.hud_visible();
        let frames = visible || (self.diagnostics.wants(Level::Warn) && self.model.foreground() && self.model.session_active());
        if self.frames_on != frames {
            if let Some(ch) = &mut self.channel { ch.send(&FromApp::Frames(frames)); }
            self.frames_on = frames;
        }
        let panel = if self.model.foreground() { self.observer.sample(now_ms) } else { None };
        self.stats.tick(self.model.prefs.stats, Sample::merge(Sample::hud(self.model.prefs.stats, visible), panel), now_ms);
        Ok(false)
    }

    fn launch_app(&mut self, app_id: &str, parameters: serde_json::Value) -> io::Result<()> {
        let channel = self.channel.as_mut().ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "the TV application service is unavailable"))?;
        self.media_seq = self.media_seq.wrapping_sub(1);
        channel.try_send(&FromApp::Launch { seq: self.media_seq, app_id: app_id.into(), parameters })
    }

    fn background_work(&mut self, inputs: &mut Vec<Input>) {
        while let Some(url) = self.art_waiting.front() {
            if !url.contains("://") {
                let checked = local_art(&self.model, self.store.root(), url);
                inputs.push(match checked { Some(path) => Input::ArtReady { url: url.clone(), path }, None => Input::ArtFailed { url: url.clone() } });
                self.art_waiting.pop_front();
                continue;
            }
            let mut artwork = None;
            for game in self.model.games() {
                for (core, kind) in [(ArtKind::Cover, Kind::Cover), (ArtKind::Hero, Kind::Hero), (ArtKind::Logo, Kind::Logo), (ArtKind::Icon, Kind::Icon)] {
                    if game.art(core).first() == Some(url) { artwork = Some(lumagame_meta::keyless_art(game, kind)); break; }
                }
                if artwork.is_some() { break; }
            }
            let artwork = artwork.unwrap_or_else(|| vec![Artwork { kind: Kind::Hero, url: url.clone(), credit: String::new() }]);
            match self.art.request(Job { key: url.clone(), artwork, spec: None }) {
                Ok(false) => break,
                Ok(true) => { self.art_waiting.pop_front(); }
                Err(_) => { inputs.push(Input::ArtFailed { url: url.clone() }); self.art_waiting.pop_front(); }
            }
        }
        while let Some(done) = self.art.poll() {
            inputs.push(match done.result.ok().and_then(|loaded| loaded.path) { Some(path) => Input::ArtReady { url: done.key, path }, None => Input::ArtFailed { url: done.key } });
        }
        if let Some(details) = &self.details {
            while let Some(id) = self.details_waiting.front() {
                let game = self.model.game(*id);
                let external = game.map(|game| game.external().into_iter().cloned().collect()).unwrap_or_default();
                let title = game.map(|game| game.title.clone()).unwrap_or_default();
                if details.jobs.try_send((*id, external, title)).is_err() { break; }
                self.details_waiting.pop_front();
            }
            while let Ok((game, hub, metadata)) = details.results.try_recv() {
                if let Some(found) = game_details(hub, metadata) { inputs.push(Input::Details { game, details: Box::new(found) }); }
            }
        }
        while let Some(notice) = self.lease.notice() { inputs.push(Input::Notice(notice)); }
        while let Some(reply) = self.controllers.poll() {
            inputs.push(match reply { ControllerReply::GameSir(result) => Input::GameSir(result), ControllerReply::Leadjoy(result) => Input::Leadjoy(result) });
        }
    }
}

fn inside(root: &Path, path: &Path) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let path = if path.is_absolute() { path.to_owned() } else { root.join(path) }.canonicalize().ok()?;
    (path.starts_with(root) && path.is_file()).then_some(path)
}

fn local_art(model: &Model, data: &Path, url: &str) -> Option<PathBuf> {
    // An account's profile picture is a file the account's engine keeps in its own cache directory.
    for engine in model.integrations.iter().filter(|e| e.targets.iter().any(|t| t.avatar.as_deref() == Some(url))) {
        if let Some(path) = inside(&data.join("cache").join(engine.manifest.id.as_str()), Path::new(url)) { return Some(path); }
    }
    for source in model.games().iter().flat_map(|game| &game.sources) {
        let art = &source.entry.art;
        if ![&art.cover, &art.hero, &art.logo, &art.icon].iter().any(|entry| entry.as_deref() == Some(url)) { continue; }
        if let Some(path) = inside(&data.join("cache").join(source.engine.as_str()), Path::new(url)) { return Some(path); }
        if source.engine.as_str() == "native" && source.entry.id.as_str().bytes().all(|c| c.is_ascii_alphanumeric() || b".-_".contains(&c)) {
            for root in ["/usr/palm/applications", "/media/developer/apps/usr/palm/applications", "/media/cryptofs/apps/usr/palm/applications"] {
                if let Some(path) = inside(&Path::new(root).join(source.entry.id.as_str()), Path::new(url)) { return Some(path); }
            }
        }
    }
    None
}

fn run(o: Options, diagnostics: &Diagnostics) -> io::Result<()> {
    let (data, temporary) = store::data_dir(&o.data)?;
    let (store, snapshot) = Store::open(data.clone())?;
    let video_full_flag = data.join(video::FULL_FLAG);
    diagnostics.configure(snapshot.diagnostics.clone());
    let (installed, notices) = host::discover(&o.engines)?;
    let mut model = Model::new(snapshot.state, snapshot.listing, installed.iter().map(|engine| engine.manifest.clone()).collect());
    model.diagnostics = snapshot.diagnostics.clone();
    let channel = o.channel_fd.map(|fd| unsafe { Channel::from_fd(fd) }).transpose()?;
    let host = Supervisor::new(installed, host::Config { data, feeder: o.feeder, runner: o.runner,
        display: Display { width: 3840, height: 2160, refresh_hz: 120, hdr10: true, codecs: vec![Codec::H264, Codec::H265, Codec::Av1] }, locale: "en-US".into(), lifecycle: channel.is_some() })?;
    let art = Loader::with_client(store.root().join("cache/art"), CacheLimits::default(), 2, Client::<Http>::new(snapshot.secrets.clone())).map_err(io::Error::other)?;
    let details = Details::new(snapshot.secrets, store.root().join("cache/hub"))?;
    let mut inputs = vec![Input::Foreground(!o.hidden)];
    inputs.extend(snapshot.notices.into_iter().chain(notices).chain(temporary).map(Input::Notice));
    let mut uplink_watch = uplink::Watch::default();
    inputs.extend(uplink_watch.poll(0, uplink::read).map(Input::Uplink));
    // Seed wall-clock time before any engine can report Running.
    model.step(Input::Tick { now_ms: 0, unix_s: unix_seconds() });
    let mut shell = Shell { model, host, store, channel, lease: Lease::new(o.helper)?, controllers: ControllerClient::new(o.helper)?, stats: OverlayClient::new(o.helper)?, frames_on: false, art, art_waiting: VecDeque::new(), details, details_waiting: VecDeque::new(), state_saved: true, media_seq: u64::MAX, diagnostics: diagnostics.clone(), observer: Observer::default(), events: Vec::new(), card: None, video_up: false, video_full_flag };
    let pads = MenuPads::new()?;
    if shell.inputs(inputs, &pads, 0)? { return Ok(()); }
    let began = Instant::now();
    std::fs::create_dir_all(&o.shm)?;
    let libmali = o.libmali.as_deref().ok_or_else(|| io::Error::other("--libmali or LUMATV_GFX_LIB must name the GL driver"))?;
    let spec = Spec { app_id: o.app_id, title: "LumaGame".into(), window_type: "_WEBOS_WINDOW_TYPE_CARD".into(), keys_back: true,
        state: (!o.hidden).then_some(FULLSCREEN), keys: true, video: true };
    match std::fs::remove_file(&o.window_id) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let (mut gles, mut window) = window::open(W, H, libmali, &spec)?;
    // Linking the engine's programs is 1.3 s of every start on the set's Mali; later starts load the kept binaries.
    if let Err(e) = gles.use_program_binaries(shell.store.root().join("cache/shaders")) { eprintln!("lumagame: shader binaries are not kept: {e}"); }
    if let Some(id) = window.video_window_id() {
        if let Some(parent) = o.window_id.parent() { std::fs::create_dir_all(parent)?; }
        std::fs::write(&o.window_id, format!("{id}\n"))?;
    } else { eprintln!("lumagame: compositor exported no video element; streaming needs a webOS video window"); }
    let (mut renderer, endpoint) = Engine::new(gles, HostConfig { width: W, height: H, refresh_hz: 120.0, shm_dir: o.shm.clone(), direct: false, gpu_sync: false,
        idle_trim: true, pages: 0, reference: false, blit: true, external: false, system_z: i32::MAX })?;
    renderer.set_transparent(true);
    let (scene, _, events) = ui::Scene::over_with(endpoint, SceneOptions { shm_dir: Some(o.shm.clone()), ..SceneOptions::default() })?;
    renderer.finish_handshake()?;
    let mut ui = Ui::new(scene);
    ui.enable_glyph_text().map_err(io::Error::other)?;
    let mut land = |ui: &mut Ui| -> io::Result<()> {
        let started = Instant::now();
        loop {
            while let Ok(event) = events.try_recv() { ui.scene.observe(&event); }
            if let Some(atlas) = ui.text_atlas() { atlas.pump(); }
            if ui.scene.has_pending() { ui.try_commit()?; }
            renderer.pump()?; renderer.apply_waiting(now_ns()); renderer.tidy();
            if ui.glyph_text_settled() && !ui.scene.has_pending() && !renderer.tidy_more() { return Ok(()); }
            if started.elapsed() > Duration::from_secs(5) { return Err(io::Error::other("the glyph atlas did not settle in 5 seconds")); }
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let mut page = GameUi::build(ui, &o.assets, &mut land)?;
    land(&mut page.ui)?;
    let mut theme = ThemeWatch::new();
    if let Some(tokens) = theme.poll(0) { page.set_host_tokens(tokens); }
    std::thread::Builder::new().name("game-swap-watch".into()).spawn(|| loop {
        std::thread::sleep(Duration::from_millis(500));
        let since = SWAP_SINCE_MS.load(Ordering::Relaxed);
        if since != 0 && (now_ns() / 1_000_000).saturating_sub(since) > 8_000 { unsafe { libc::_exit(1); } }
    })?;
    let mut ready = false;
    let mut want_frame = true;
    let mut painted_ui = true;
    // Not fatal: without the socket LumaTV's task manager has no card or quit hook for LumaGame, and the app runs as before.
    let hook = Hook::start(&spec.app_id).map_err(|e| eprintln!("lumagame: the task-manager hook is off: {e}")).ok();
    let mut signalled = false;
    let mut fds = Vec::new();
    let mut frame = 0u64;
    let mut last_flash = None;
    let mut trace = LifecycleTrace::enabled(std::env::var_os("LUMAGAME_LIFECYCLE_TRACE").as_deref());
    let mut remote_trace: Option<LifecycleTrace> = None;
    let mut render_sample: Option<(u64, u32, u32, u32)> = None;
    loop {
        let now_ms = began.elapsed().as_millis() as u64;
        let mut trace_events = 0u8;
        let mut trace_window_state = None;
        if shell.model.foreground() {
            if let Some(tokens) = theme.poll(now_ms) { page.set_host_tokens(tokens); }
        }
        let mut inputs = vec![Input::Tick { now_ms, unix_s: unix_seconds() }];
        if let Some(change) = uplink_watch.poll(now_ms, uplink::read) { inputs.push(Input::Uplink(change)); }
        if shell.model.in_diagnostics() {
            inputs.push(Input::DiagnosticsStatus(shell.diagnostics.status()));
        }
        if END.load(Ordering::Relaxed) && !signalled { inputs.push(Input::Close); signalled = true; }
        for command in hook.as_ref().map(Hook::poll).unwrap_or_default() {
            match command {
                // the same path as SAM's close and SIGTERM: the model stops any session, saves state and exits
                HookCommand::Quit => if !signalled { inputs.push(Input::Close); signalled = true; },
                HookCommand::Snapshot { path, w, h, reply } => { let _ = reply.send(hook_snapshot(&renderer, frame, &path, w, h)); }
            }
        }
        for event in window.events() {
            match event {
                Event::State(state) => { trace_events |= 2; trace_window_state = Some(state); if let Some(ch) = &mut shell.channel { ch.send(&FromApp::State(state)); } inputs.push(Input::Foreground(state != MINIMIZED)); }
                Event::Close => { trace_events |= 4; inputs.push(Input::Close); }
                Event::Key { code, pressed } => if let Some(key) = Key::from_code(code) { inputs.push(Input::Key { key, down: pressed }); },
                // the keys come back when the player closes (Back, its own end or its dwell)
                Event::Focus(has) => if has { shell.video_up = false; },
            }
        }
        // One stat a pass, only while the player is up; the pass wakes every VIDEO_FLAG_MS for it (below).
        let video_full = shell.video_up && shell.video_full_flag.exists();
        if video_full != shell.model.video_full { inputs.push(Input::VideoFull(video_full)); }
        if let Some(channel) = &mut shell.channel {
            match channel.read() {
                Ok(messages) => for message in messages {
                    shell.host.lifecycle(&message);
                    match message {
                        ToApp::Relaunch { .. } => { trace_events |= 1; window.request_state(FULLSCREEN); inputs.push(Input::Foreground(true)); want_frame = true; }
                        ToApp::Close { .. } => { trace_events |= 4; inputs.push(Input::Close); }
                        ToApp::Launched { seq, success: false, message, .. } if seq == shell.media_seq => inputs.push(Input::Notice(message)),
                        ToApp::Frames(frames) => inputs.push(Input::Frames(frames)),
                        _ => {}
                    }
                }
                Err(e) => { shell.host.channel_failed(e.to_string()); shell.channel = None; inputs.push(Input::Close); }
            }
        }
        pads.poll(&mut inputs, frame + 1);
        for event in shell.host.poll(now_ms) {
            inputs.push(match event { HostEvent::Engine { engine, message } => Input::Engine { engine, message }, HostEvent::Gone(id) => Input::EngineGone(id), HostEvent::Notice(why) => Input::Notice(why) });
        }
        shell.background_work(&mut inputs);
        while let Some((setting, display)) = shell.stats.poll() {
            inputs.push(Input::StatsSetting(setting));
            if let Some(display) = display { inputs.push(Input::Display(display)); }
        }
        shell.card = page.frame_of(shell.model.view.focus).map(|f| video::card(f.x, f.y, f.w, f.h));
        let exiting = shell.inputs(inputs, &pads, now_ms)?;
        let visible = shell.model.wants_frames();
        if shell.diagnostics.enabled() {
            let trace = remote_trace.get_or_insert_with(LifecycleTrace::default);
            let state = TraceState::of(&shell.model);
            let draw_event = if trace.last != Some(state) { trace_events | 1 } else { trace_events };
            if let Some(seq) = trace.observe(state, draw_event) {
                shell.diagnostics.emit(DiagnosticEvent::Lifecycle { ms: now_ms, page: page_name(state.page), foreground: state.foreground, phase: state.phase, visible: state.visible, sequence: seq });
            }
        } else { remote_trace = None; render_sample = None; }
        if let Some(trace) = &mut trace {
            let state = TraceState::of(&shell.model);
            if let Some(seq) = trace.observe(state, trace_events) {
                eprintln!("lumagame-trace: ms={now_ms} seq={seq} events={trace_events} window_state={trace_window_state:?} page={:?} foreground={} phase={} wants_frames={visible} want_frame={want_frame} painted_ui={painted_ui} eligible={} frame={frame} exiting={exiting}", state.page, state.foreground, state.phase, state.foreground && (visible || painted_ui));
            }
        }
        if exiting { return Ok(()); }
        // Running keeps only the opt-in HUD; Off commits a transparent scene once and leaves the window idle.
        if shell.model.foreground() && (visible || painted_ui) {
            while let Ok(event) = events.try_recv() { page.ui.scene.observe(&event); }
            if let Some(atlas) = page.ui.text_atlas() { atlas.pump(); }
            let flash = shell.model.flash_press();
            let changed = page.show(&shell.model)?;
            if page.ui.scene.has_pending() { page.ui.try_commit()?; }
            if renderer.pump()?.closed { return Err(io::Error::other("the renderer closed")); }
            let now = now_ns();
            let due = match renderer.demand() { Demand::EveryRefresh => true, Demand::At(at) => now >= at, Demand::Idle => false };
            if let Some(seq) = trace.as_mut().and_then(LifecycleTrace::decision) {
                let commits = renderer.has_commits();
                let redraw = renderer.redraw_wanted();
                let draw = want_frame || changed || commits || redraw || due;
                eprintln!("lumagame-trace: ms={now_ms} seq={seq} decision draw={draw} want_frame={want_frame} changed={changed} commits={commits} redraw={redraw} due={due} frame={frame}");
            }
            if want_frame || changed || renderer.has_commits() || renderer.redraw_wanted() || due {
                want_frame = false;
                let cost = renderer.draw(now);
                if shell.diagnostics.enabled() {
                    let sample = render_sample.get_or_insert((now_ms, 0, 0, 0));
                    sample.1 = sample.1.saturating_add(1);
                    sample.2 = sample.2.max(cost.cpu_us);
                    sample.3 = sample.3.max(cost.draw_us);
                    if now_ms.saturating_sub(sample.0) >= 1000 {
                        shell.diagnostics.emit(DiagnosticEvent::Render { ms: now_ms, frames: sample.1, max_cpu_us: sample.2, max_draw_us: sample.3 });
                        render_sample = None;
                    }
                }
                if let Some(seq) = remote_trace.as_ref().and_then(|t| t.pending) {
                    shell.diagnostics.emit(DiagnosticEvent::Draw { ms: now_ms, sequence: seq, frame: frame + 1 });
                }
                if let Some(error) = cost.gl_error { eprintln!("lumagame: GL error {error:#x}"); }
                let trace_swap_start = trace.as_ref().and_then(|t| t.pending).map(|_| Instant::now());
                SWAP_SINCE_MS.store((now_ns() / 1_000_000).max(1), Ordering::Relaxed);
                let swap = renderer.gles().swap_buffers();
                SWAP_SINCE_MS.store(0, Ordering::Relaxed);
                if let Some(seq) = remote_trace.as_mut().and_then(LifecycleTrace::swapped) {
                    shell.diagnostics.emit(DiagnosticEvent::Swap { ms: began.elapsed().as_millis() as u64, sequence: seq, frame: frame + 1, success: swap.is_ok() });
                }
                if let Some(seq) = trace.as_mut().and_then(LifecycleTrace::swapped) {
                    let elapsed_ms = trace_swap_start.map_or(0, |start| start.elapsed().as_millis());
                    eprintln!("lumagame-trace: ms={} seq={seq} swap={swap:?} elapsed_ms={elapsed_ms} frame={}", began.elapsed().as_millis(), frame + 1);
                }
                if swap.is_err() && shell.diagnostics.enabled() {
                    shell.diagnostics.emit(DiagnosticEvent::Error { ms: now_ms, kind: "swap_failed" });
                }
                swap?;
                frame += 1;
                if let Some((press, shown_frame)) = flash {
                    if last_flash != Some(press) {
                        let shown = now_ns();
                        eprintln!("lumagame: flash press_ns={press} frame={shown_frame} shown_ns={shown} delay_ms={:.2}", shown.saturating_sub(press) as f64 / 1e6);
                    }
                }
                last_flash = flash.map(|(press, _)| press);
                renderer.frame_done(now_ns(), &cost); renderer.tidy(); renderer.idle_moment(true);
                painted_ui = visible;
                if !ready { ready = true; if let Some(ch) = &mut shell.channel { ch.send(&FromApp::Ready); } }
                continue;
            }
            renderer.apply_waiting(now);
            if renderer.tidy_more() { renderer.tidy(); continue; }
            renderer.idle_moment(false);
        } else if !shell.model.foreground() { want_frame = true; painted_ui = true; }
        let mut wait_ms = shell.host.wait_ms(now_ms).min(shell.lease.wait_ms(now_ms)).min(shell.stats.wait_ms(now_ms));
        if page.art_busy() || shell.art.inflight() > 0 || !shell.art_waiting.is_empty() { wait_ms = wait_ms.min(16); }
        if let Some(ms) = page.wake_ms(now_ms) { wait_ms = wait_ms.min(ms); }
        if let Some(ms) = shell.model.wake_in(now_ms) { wait_ms = wait_ms.min(ms); }
        if shell.video_up { wait_ms = wait_ms.min(VIDEO_FLAG_MS); }
        if shell.channel.as_ref().is_some_and(Channel::pending) { wait_ms = wait_ms.min(16); }
        if shell.model.pad_mode() != PadMode::Menu || shell.model.gamesir.pending || shell.model.leadjoy.pending { wait_ms = wait_ms.min(50); }
        if visible {
            let demand = renderer.demand();
            if let Demand::At(at) = demand { wait_ms = wait_ms.min(at.saturating_sub(now_ns()) / 1_000_000); }
            if let Some(at) = renderer.idle_wake_at(demand == Demand::Idle) { wait_ms = wait_ms.min(at.saturating_sub(now_ns()) / 1_000_000); }
        }
        fds.clear(); fds.push(pads.receive.as_raw_fd()); fds.extend(hook.as_ref().map(Hook::fd));
        fds.extend(shell.channel.as_ref().map(Channel::fd)); shell.host.fds(&mut fds);
        window.wait_fds(wait_ms.min(1000) as i32, &fds)?;
    }
}

/// The task manager's card: the UI's last presented frame (the render target keeps it whatever the window swapped since). A stream or
/// video on the hardware layer is not in it, so over a playing game the card is the UI's frame, mostly transparent and so black.
fn hook_snapshot(renderer: &Engine, frame: u64, path: &Path, w: u32, h: u32) -> Result<(), String> {
    if frame == 0 { return Err("no frame has been presented yet".into()); }
    let rows = renderer.read_page(0);
    hook::write_snapshot(path, &hook::scale(&rows, W, H, w, h)).map_err(|e| e.to_string())
}

struct ThemeWatch {
    path: PathBuf,
    stamp: Option<(SystemTime, u64)>,
    first: bool,
    due: u64,
}

impl ThemeWatch {
    fn new() -> Self {
        let data = std::env::var_os("LUMATV_DATA_DIR").map(PathBuf::from).unwrap_or_else(|| "/var/lib/lumatv".into());
        Self { path: data.join("theme/theme.json"), stamp: None, first: true, due: 0 }
    }

    fn poll(&mut self, now_ms: u64) -> Option<lumatv_tokens::Tokens> {
        if now_ms < self.due { return None; }
        self.due = now_ms.saturating_add(1_000);
        let stamp = std::fs::metadata(&self.path).ok().and_then(|m| Some((m.modified().ok()?, m.len())));
        if !std::mem::take(&mut self.first) && stamp == self.stamp { return None; }
        self.stamp = stamp;
        let Some((_, len)) = stamp else { return Some(lumatv_tokens::Tokens::shipped()); };
        if len > 1 << 20 { return None; }
        let bytes = std::fs::read(&self.path).ok()?;
        let value = serde_json::from_slice(&bytes).ok()?;
        Some(lumatv_tokens::Tokens::from_json(&value))
    }
}

fn unix_seconds() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_trace_is_opt_in_and_logs_transitions_not_ticks() {
        use std::ffi::OsStr;
        use lumatv_rustui::game::model::Page;
        assert!(LifecycleTrace::enabled(None).is_none());
        assert!(LifecycleTrace::enabled(Some(OsStr::new("0"))).is_none());
        let mut trace = LifecycleTrace::enabled(Some(OsStr::new("1"))).unwrap();
        let state = TraceState { page: Page::Game, foreground: false, phase: "done", visible: false };
        assert_eq!(trace.observe(state, 0), Some(1));
        assert_eq!(trace.observe(state, 0), None);
        assert_eq!(trace.observe(state, 1), Some(2));
        assert_eq!(trace.decision(), Some(2));
        assert_eq!(trace.decision(), None);
        assert_eq!(trace.swapped(), Some(2));
        assert_eq!(trace.swapped(), None);
        assert_eq!(trace.observe(TraceState { foreground: true, visible: true, ..state }, 2), Some(3));
        assert_eq!(trace.decision(), Some(3));
        assert_eq!(trace.swapped(), Some(3));
    }

    #[test]
    fn menu_ok_activates_on_release_and_is_silent_when_not_grabbed() {
        let (send, receive) = UnixDatagram::pair().unwrap();
        receive.set_nonblocking(true).unwrap();
        let sink = MenuSink { active: AtomicBool::new(true), mode: AtomicU8::new(PadMode::Menu as u8), connected: AtomicU32::new(0), keys: Mutex::new(std::array::from_fn(|_| PadKeys::default())), lab: Mutex::new(LabBuffer::default()), send };
        let report = |buttons| PadEvent::Report { slot: 2, state: PadState { buttons, ..PadState::default() }, press: None };
        let mut bytes = [0; 3];
        sink.pad_event(1, report(button::A));
        assert_eq!(receive.recv(&mut bytes).unwrap(), 3);
        assert_eq!(bytes, [4, 1, 2], "key, down, and the pad's slot");
        sink.pad_event(1, report(button::A));
        assert!(receive.recv(&mut bytes).is_err());
        sink.pad_event(1, report(0));
        receive.recv(&mut bytes).unwrap();
        assert_eq!(bytes, [4, 0, 2]);
        sink.active.store(false, Ordering::Release);
        sink.pad_event(1, report(button::B));
        sink.pad_event(1, report(0));
        assert!(receive.recv(&mut bytes).is_err());
    }

    #[test]
    fn menu_y_releases_so_the_menu_key_acts_and_the_next_press_is_not_swallowed() {
        let (send, receive) = UnixDatagram::pair().unwrap();
        receive.set_nonblocking(true).unwrap();
        let sink = MenuSink { active: AtomicBool::new(true), mode: AtomicU8::new(PadMode::Menu as u8), connected: AtomicU32::new(0), keys: Mutex::new(std::array::from_fn(|_| PadKeys::default())), lab: Mutex::new(LabBuffer::default()), send };
        let report = |buttons| PadEvent::Report { slot: 0, state: PadState { buttons, ..PadState::default() }, press: None };
        let mut bytes = [0; 3];
        let menu = KEYS.iter().position(|k| *k == Key::Menu).unwrap() as u8;
        sink.pad_event(1, report(button::Y));
        receive.recv(&mut bytes).unwrap();
        assert_eq!(bytes, [menu, 1, 0]);
        sink.pad_event(1, report(0));
        receive.recv(&mut bytes).unwrap();
        assert_eq!(bytes, [menu, 0, 0]);
    }

    #[test]
    fn a_held_direction_sends_its_release_when_it_changes_ends_or_the_pad_goes() {
        let (send, receive) = UnixDatagram::pair().unwrap();
        receive.set_nonblocking(true).unwrap();
        let sink = MenuSink { active: AtomicBool::new(true), mode: AtomicU8::new(PadMode::Menu as u8), connected: AtomicU32::new(0), keys: Mutex::new(std::array::from_fn(|_| PadKeys::default())), lab: Mutex::new(LabBuffer::default()), send };
        let report = |buttons| PadEvent::Report { slot: 1, state: PadState { buttons, ..PadState::default() }, press: None };
        let drain = || {
            let mut bytes = [0; 3];
            std::iter::from_fn(|| receive.recv(&mut bytes).ok().map(|_| bytes)).collect::<Vec<_>>()
        };
        let (up, down) = (0, 1);
        sink.pad_event(1, report(button::DPAD_UP));
        assert_eq!(drain(), [[up, 1, 1]]);
        sink.pad_event(1, report(button::DPAD_DOWN));
        assert_eq!(drain(), [[up, 0, 1], [down, 1, 1]], "the old direction lets go before the new one presses");
        sink.pad_event(1, report(0));
        assert_eq!(drain(), [[down, 0, 1]]);
        sink.pad_event(1, report(button::DPAD_UP));
        drain();
        sink.pad_event(1, PadEvent::Disconnected { slot: 1 });
        assert!(drain().contains(&[up, 0, 1]), "a pad that goes lets go of what it held");
    }

    #[test]
    fn lab_owns_sticks_capture_owns_all_keys_and_reports_share_one_wake() {
        let (send, receive) = UnixDatagram::pair().unwrap();
        receive.set_nonblocking(true).unwrap();
        let sink = MenuSink { active: AtomicBool::new(true), mode: AtomicU8::new(PadMode::Lab as u8), connected: AtomicU32::new(0), keys: Mutex::new(std::array::from_fn(|_| PadKeys::default())), lab: Mutex::new(LabBuffer::default()), send };
        let report = |buttons| PadEvent::Report { slot: 0, state: PadState { left_x: i16::MAX, left_y: i16::MAX, buttons, ..PadState::default() }, press: None };
        let mut bytes = [0; 3];
        sink.pad_event(1, report(0));
        assert!(receive.recv(&mut bytes).is_err());
        sink.mode.store(PadMode::Captured as u8, Ordering::Release);
        sink.pad_event(1, report(button::A | button::B | button::DPAD_UP));
        sink.pad_event(1, PadEvent::Guide { slot: 0, pressed: true });
        sink.pad_event(1, report(0));
        assert!(receive.recv(&mut bytes).is_err());
        sink.raw(0, &RawReport { kernel_ns: 1, ..Default::default() });
        sink.raw(0, &RawReport { kernel_ns: 2, ..Default::default() });
        assert_eq!(receive.recv(&mut bytes).unwrap(), 1);
        assert!(receive.recv(&mut bytes).is_err());
        assert_eq!(sink.lab.lock().unwrap().reports.len(), 2);
        sink.mode.store(PadMode::Menu as u8, Ordering::Release);
        sink.raw(0, &RawReport::default());
        assert_eq!(sink.lab.lock().unwrap().reports.len(), 2);
    }

    #[test]
    fn engine_art_cannot_escape_its_cache_through_a_symlink_or_parent_path() {
        let root = std::env::var_os("CODY_DATA_SCRATCH").or_else(|| std::env::var_os("TMPDIR")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let dir = tempfile::tempdir_in(root).unwrap();
        let cache = dir.path().join("cache");
        std::fs::create_dir(&cache).unwrap();
        std::fs::write(dir.path().join("secrets.json"), b"private").unwrap();
        std::fs::write(cache.join("cover"), b"image").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secrets.json"), cache.join("link")).unwrap();
        assert!(inside(&cache, Path::new("cover")).is_some());
        assert!(inside(&cache, Path::new("../secrets.json")).is_none());
        assert!(inside(&cache, Path::new("link")).is_none());
    }
}
