//! An in-process compositor (software renderer, pure-Rust backend) on one end of a socketpair and a
//! `wayland-client` on the other. The server runs on its own thread and ticks every 4 ms; tests reach into its
//! `State` through [`Server::eval`].

#![allow(dead_code)]

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_callback::WlCallback;
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_keyboard::WlKeyboard;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shell::WlShell;
use wayland_client::protocol::wl_shell_surface::{self, WlShellSurface};
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_subcompositor::WlSubcompositor;
use wayland_client::protocol::wl_subsurface::WlSubsurface;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop, event_created_child};
use wayland_server::Display;

use lumatv_compositor::config::Config;
use lumatv_compositor::luna;
use lumatv_compositor::output::Output;
use lumatv_compositor::render::software::Software;
use lumatv_compositor::server::state::{ClientEvent, ClientHandler, State};

use lumatv_wl_protocols::input_method::client::input_method::{self, InputMethod};
use lumatv_wl_protocols::input_method::client::input_method_context::{self, InputMethodContext};
use lumatv_wl_protocols::input_method::client::input_panel::InputPanel;
use lumatv_wl_protocols::input_method::client::input_panel_surface::InputPanelSurface;
use lumatv_wl_protocols::mali_buffer_sharing::client::mali_buffer_sharing::{self, MaliBufferSharing};
use lumatv_wl_protocols::text::client::text_model::{self, TextModel};
use lumatv_wl_protocols::text::client::text_model_factory::TextModelFactory;
use lumatv_wl_protocols::webos_foreign::client::wl_webos_exported::{self, WlWebosExported};
use lumatv_wl_protocols::webos_foreign::client::wl_webos_foreign::WlWebosForeign;
use lumatv_wl_protocols::webos_foreign::client::wl_webos_imported::{self, WlWebosImported};
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell::WlWebosShell;
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell_surface::{self, WlWebosShellSurface};
use lumatv_wl_protocols::webos_surface_group::client::wl_webos_surface_group::WlWebosSurfaceGroup;
use lumatv_wl_protocols::webos_surface_group::client::wl_webos_surface_group_compositor::WlWebosSurfaceGroupCompositor;
use lumatv_wl_protocols::webos_surface_group::client::wl_webos_surface_group_layer::WlWebosSurfaceGroupLayer;

/// The `display/setDisplayWindow` payloads the compositor has sent, oldest first.
pub fn placements(s: &mut State) -> Vec<serde_json::Value> {
    s.luna_sent().iter().filter(|(u, _)| u.ends_with("/setDisplayWindow")).map(|(_, p)| p.clone()).collect()
}

/// What videooutputd takes (TV round 15: `{"errorCode":4,"errorMessage":"Invalid Parameters"}` for every placement that broke this, the wallpaper's
/// plane black as a result): a `displayOutput`, a `context`, and a `sourceInput` only together with its `originalInput`.
pub fn assert_placements_valid(placements: &[serde_json::Value]) {
    for p in placements {
        assert!(p.get("displayOutput").is_some_and(|d| d.get("width").is_some() && d.get("height").is_some()), "no displayOutput: {p}");
        assert!(p.get("context").is_some_and(|c| c.as_str().is_some_and(|c| !c.is_empty())), "no context: {p}");
        assert_eq!(p.get("sourceInput").is_some(), p.get("originalInput").is_some(), "a source goes with its original input, and only with it: {p}");
    }
}

type Job = Box<dyn FnOnce(&mut State) + Send>;

pub struct Server {
    jobs: Sender<Job>,
    thread: Option<JoinHandle<()>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    /// While false the server dispatches but does not tick (frames are drawn only when something forces one).
    pub ticking: Arc<std::sync::atomic::AtomicBool>,
    pub client_stream: Option<UnixStream>,
    /// The connect/disconnect queue every client of this server reports into.
    events: Arc<Mutex<Vec<ClientEvent>>>,
}

pub type OutputFactory = Box<dyn FnOnce() -> Box<dyn Output> + Send>;

impl Server {
    pub fn start() -> Server {
        Self::start_with(Box::new(|| Box::new(Software::new(1920, 1080))))
    }

    /// Starts the server with an output made on the server thread (a GL context is bound to its thread).
    pub fn start_with(output: OutputFactory) -> Server {
        let (server_end, client_end) = UnixStream::pair().unwrap();
        let (jobs, rx) = channel::<Job>();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ticking = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let events = Arc::new(Mutex::new(Vec::new()));
        let (stop2, ticking2, events2) = (stop.clone(), ticking.clone(), events.clone());
        let thread = std::thread::spawn(move || run_server(server_end, rx, stop2, ticking2, output, events2));
        Server { jobs, thread: Some(thread), stop, ticking, client_stream: Some(client_end), events }
    }

    /// Runs `f` on the server thread with its state and returns the result.
    pub fn eval<R: Send + 'static>(&self, f: impl FnOnce(&mut State) -> R + Send + 'static) -> R {
        let (tx, rx) = channel();
        self.jobs
            .send(Box::new(move |s| {
                let _ = tx.send(f(s));
            }))
            .unwrap();
        // How long a job takes is the work's (a hundred ticks and read-backs of the software output) times the machine's load, so there is no fixed
        // bound that holds on a busy machine: wait while the server thread is alive, and let a job that panics (its sender dropped) fail here. The
        // cap only ends a deadlock.
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(r) => return r,
                Err(RecvTimeoutError::Disconnected) => panic!("the server thread ended before it answered"),
                Err(RecvTimeoutError::Timeout) => assert!(Instant::now() < deadline, "the server thread did not answer in 300 s"),
            }
        }
    }

    pub fn summary(&self) -> String {
        self.eval(|s| s.summary())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// What the output shows once the scene's pending changes are drawn: one tick on the server thread, then the read-back. The loop ticks every
/// 4 ms by itself, but a loaded machine leaves that thread late, and a sleep before a read-back races with it.
pub fn drawn(s: &mut State) -> lumatv_compositor::output::Image {
    s.tick();
    s.output.capture().unwrap()
}

fn run_server(
    stream: UnixStream,
    jobs: Receiver<Job>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    ticking: Arc<std::sync::atomic::AtomicBool>,
    output: OutputFactory,
    events: Arc<Mutex<Vec<ClientEvent>>>,
) {
    let mut display: Display<State> = Display::new().unwrap();
    let dh = display.handle();
    // Isolate persistent compositor choices from the host and other tests.
    static SERVERS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let data_dir = std::env::temp_dir().join(format!("lumatv-harness-{}-{}", std::process::id(), SERVERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
    let config = Config { frames: None, data_dir, ..Config::default() };
    State::create_globals(&dh, true);
    let output = output();
    let mut state = State::new(dh, config, output, Box::new(luna::Null::default()));
    // The tests read a client's frame callback as its frame's draw: a card's goes out at the draw here, as under
    // `ctl callbacks clients early`; on the set it goes out with the commit (`tests/flips.rs` sets each case).
    state.client_callbacks = lumatv_compositor::server::state::ClientCallbacks::Draw;
    display.handle().insert_client(stream, Arc::new(ClientHandler { events: events.clone() })).unwrap();
    let mut last_tick = Instant::now();
    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
        let fd = display.backend().poll_fd();
        // the display, and the system UI app's socket when there is one (as the main loop's epoll has it)
        let mut pfds = [
            libc::pollfd { fd: std::os::fd::AsRawFd::as_raw_fd(&fd), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: state.output.ui_fd().unwrap_or(-1), events: libc::POLLIN, revents: 0 },
        ];
        // SAFETY: two pollfds (a negative fd is skipped by poll), 2 ms timeout.
        unsafe { libc::poll(pfds.as_mut_ptr(), 2, 2) };
        if pfds[0].revents != 0 {
            display.dispatch_clients(&mut state).unwrap();
        }
        if pfds[1].revents != 0 && state.ui_wake() {
            state.tick();
            last_tick = Instant::now();
        }
        while let Ok(job) = jobs.try_recv() {
            job(&mut state);
        }
        if last_tick.elapsed() >= Duration::from_millis(4) && ticking.load(std::sync::atomic::Ordering::SeqCst) {
            state.tick();
            last_tick = Instant::now();
        }
        state.drain_client_events(&events);
        display.flush_clients().unwrap();
    }
}

/// The test client: the globals every LG client binds, plus what the tests collect from events.
#[derive(Default)]
pub struct ClientState {
    pub frames_done: u32,
    pub releases: Vec<u32>,
    pub configures: Vec<(i32, i32)>,
    pub shell_states: Vec<u32>,
    /// `state_changed` by the shell surface it came on (its protocol id), in order.
    pub states_of: Vec<(u32, u32)>,
    pub positions: Vec<(i32, i32)>,
    /// `exposed` by the shell surface it came on: the ints of the array, closing -1 included.
    pub exposed_of: Vec<(u32, Vec<i32>)>,
    pub closes: u32,
    pub window_ids: Vec<(String, u32)>,
    pub punchthrough_attached: Vec<String>,
    pub destination_changed: Vec<(u32, u32)>,
    pub alloc_devices: Vec<String>,
    pub keyboard_enters: u32,
    /// The keys the last `enter` carried.
    pub enter_keys: Vec<u32>,
    pub keys: Vec<(u32, u32)>,
    /// `(wl_keyboard protocol id, key)` per key event, to tell the seats apart.
    pub keys_by_keyboard: Vec<(u32, u32)>,
    pub pointer_enters: Vec<(f64, f64)>,
    pub pointer_leaves: u32,
    pub motions: Vec<(f64, f64)>,
    pub buttons: Vec<(u32, u32)>,
    pub cursor_visibility: Vec<u32>,
    /// `wl_seat.capabilities` and `wl_seat.name`, every seat bound.
    pub seat_capabilities: Vec<u32>,
    pub seat_names: Vec<String>,
    /// `wl_webos_seat.info`: id, device name, designator, capabilities.
    pub webos_seat_infos: Vec<(u32, String, u32, u32)>,
    /// `wl_starfish_pointer.timer_value`.
    pub starfish_timer_values: Vec<u32>,
    /// What the text model and the input method were told (`ime.rs`).
    pub ime: ImeLog,
}

/// The keyboard path as a client sees it: an app's text model and an input method's objects both log here.
#[derive(Default)]
pub struct ImeLog {
    pub model_enters: u32,
    pub model_leaves: u32,
    pub commits: Vec<String>,
    pub preedits: Vec<String>,
    pub deletes: Vec<(i32, u32)>,
    pub keysyms: Vec<(u32, u32)>,
    pub panel_states: Vec<u32>,
    pub panel_rects: Vec<(i32, i32, u32, u32)>,
    /// The input method side: the contexts it was given, how often it was told to show or hide the panel, and which were deactivated.
    pub contexts: Vec<InputMethodContext>,
    pub activations: u32,
    pub deactivated: Vec<u32>,
    pub shows: u32,
    pub hides: u32,
    /// What a context was told, one word per event with its arguments.
    pub context_events: Vec<String>,
}

pub struct Client {
    pub conn: Connection,
    pub queue: EventQueue<ClientState>,
    pub qh: QueueHandle<ClientState>,
    pub globals: GlobalList,
    pub state: ClientState,
    pub compositor: WlCompositor,
    pub shell: WlShell,
    pub shm: WlShm,
    pub webos_shell: WlWebosShell,
    pub foreign: WlWebosForeign,
    pub groups: WlWebosSurfaceGroupCompositor,
    pub subcompositor: WlSubcompositor,
    pub seat: WlSeat,
}

impl Client {
    pub fn connect(server: &mut Server) -> Client {
        let stream = server.client_stream.take().expect("one client per harness");
        Self::connect_stream(stream)
    }

    /// Another client of the same server, inserted on the server thread (its events go through the same queue as
    /// the first's, so its disconnect is handled like a real one).
    pub fn connect_extra(server: &mut Server) -> Client {
        let (server_end, client_end) = UnixStream::pair().unwrap();
        let events = server.events.clone();
        server.eval(move |s| {
            s.dh.insert_client(server_end, Arc::new(ClientHandler { events })).unwrap();
        });
        Self::connect_stream(client_end)
    }

    fn connect_stream(stream: UnixStream) -> Client {
        let conn = Connection::from_socket(stream).unwrap();
        let (globals, mut queue) = registry_queue_init::<ClientState>(&conn).unwrap();
        let qh = queue.handle();
        let mut state = ClientState::default();
        let compositor: WlCompositor = globals.bind(&qh, 1..=4, ()).unwrap();
        let shell: WlShell = globals.bind(&qh, 1..=1, ()).unwrap();
        let shm: WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
        let webos_shell: WlWebosShell = globals.bind(&qh, 1..=1, ()).unwrap();
        let foreign: WlWebosForeign = globals.bind(&qh, 1..=1, ()).unwrap();
        let groups: WlWebosSurfaceGroupCompositor = globals.bind(&qh, 1..=1, ()).unwrap();
        let subcompositor: WlSubcompositor = globals.bind(&qh, 1..=1, ()).unwrap();
        let seat: WlSeat = globals.bind(&qh, 1..=4, ()).unwrap();
        queue.roundtrip(&mut state).unwrap();
        Client { conn, queue, qh, globals, state, compositor, shell, shm, webos_shell, foreign, groups, subcompositor, seat }
    }

    pub fn roundtrip(&mut self) {
        self.queue.roundtrip(&mut self.state).unwrap();
    }

    /// Dispatches until `pred` holds or `timeout` passes.
    pub fn wait_until(&mut self, timeout: Duration, mut pred: impl FnMut(&ClientState) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if pred(&self.state) {
                return true;
            }
            self.conn.flush().unwrap();
            if let Some(guard) = self.queue.prepare_read() {
                let fd = guard.connection_fd();
                let mut pfd = libc::pollfd { fd: std::os::fd::AsRawFd::as_raw_fd(&fd), events: libc::POLLIN, revents: 0 };
                // SAFETY: one pollfd, short timeout.
                unsafe { libc::poll(&mut pfd, 1, 5) };
                if pfd.revents != 0 {
                    let _ = guard.read();
                } else {
                    drop(guard);
                }
            }
            self.queue.dispatch_pending(&mut self.state).unwrap();
        }
        pred(&self.state)
    }

    pub fn text_model_factory(&mut self) -> TextModelFactory {
        self.globals.bind(&self.qh, 1..=1, ()).unwrap()
    }

    pub fn input_method(&mut self) -> InputMethod {
        self.globals.bind(&self.qh, 2..=2, ()).unwrap()
    }

    pub fn input_panel(&mut self) -> InputPanel {
        self.globals.bind(&self.qh, 1..=1, ()).unwrap()
    }

    pub fn input_manager(&mut self) -> lumatv_wl_protocols::webos_input_manager::client::wl_webos_input_manager::WlWebosInputManager {
        self.globals.bind(&self.qh, 1..=1, ()).unwrap()
    }

    pub fn starfish_pointer(&mut self) -> lumatv_wl_protocols::starfish::client::wl_starfish_pointer::WlStarfishPointer {
        self.globals.bind(&self.qh, 1..=1, ()).unwrap()
    }

    pub fn global_names(&self) -> Vec<String> {
        self.globals.contents().with_list(|l| l.iter().map(|g| g.interface.clone()).collect())
    }

    /// A card window: surface + wl_shell toplevel + webOS shell surface of the given type.
    pub fn card(&mut self, window_type: &str, title: &str) -> Window {
        self.card_as(window_type, title, &format!("org.test.{}", title.to_lowercase()))
    }

    /// A card window of the app `app_id`.
    pub fn card_as(&mut self, window_type: &str, title: &str, app_id: &str) -> Window {
        let surface = self.compositor.create_surface(&self.qh, ());
        let shell_surface = self.shell.get_shell_surface(&surface, &self.qh, ());
        shell_surface.set_title(title.into());
        shell_surface.set_toplevel();
        let webos = self.webos_shell.get_shell_surface(&surface, &self.qh, ());
        webos.set_property("_WEBOS_WINDOW_TYPE".into(), window_type.into());
        webos.set_property("appId".into(), app_id.into());
        Window { surface, shell_surface, webos }
    }

    /// An shm buffer filled with one premultiplied ARGB value.
    pub fn shm_buffer(&mut self, w: i32, h: i32, argb: u32) -> WlBuffer {
        self.shm_pattern(w, h, move |_, _| argb)
    }

    /// An shm buffer whose pixel at (x, y) is `pixel(x, y)`, premultiplied ARGB.
    pub fn shm_pattern(&mut self, w: i32, h: i32, pixel: impl Fn(i32, i32) -> u32) -> WlBuffer {
        self.shm_pattern_in(w, h, Format::Argb8888, pixel)
    }

    /// As [`Client::shm_pattern`], in `format` (Xrgb8888: the alpha byte is whatever `pixel` puts there and means nothing).
    pub fn shm_pattern_in(&mut self, w: i32, h: i32, format: Format, pixel: impl Fn(i32, i32) -> u32) -> WlBuffer {
        let stride = w * 4;
        let size = stride * h;
        // SAFETY: memfd for the test; sized and mapped below.
        let fd = unsafe { libc::memfd_create(c"test-shm".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(fd >= 0);
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let file = std::fs::File::from(fd.try_clone().unwrap());
        file.set_len(size as u64).unwrap();
        let mut bytes = Vec::with_capacity(size as usize);
        for y in 0..h {
            for x in 0..w {
                bytes.extend_from_slice(&pixel(x, y).to_le_bytes());
            }
        }
        std::io::Write::write_all(&mut &file, &bytes).unwrap();
        let pool = self.shm.create_pool(fd.as_fd(), size, &self.qh, ());
        let buffer = pool.create_buffer(0, w, h, stride, format, &self.qh, ());
        pool.destroy();
        buffer
    }

    pub fn region(&mut self, x: i32, y: i32, w: i32, h: i32) -> WlRegion {
        let r = self.compositor.create_region(&self.qh, ());
        r.add(x, y, w, h);
        r
    }

    /// Attaches, damages everything, asks for a frame and commits.
    pub fn present(&mut self, surface: &WlSurface, buffer: &WlBuffer) -> WlCallback {
        surface.attach(Some(buffer), 0, 0);
        surface.damage(0, 0, i32::MAX, i32::MAX);
        let cb = surface.frame(&self.qh, ());
        surface.commit();
        self.conn.flush().unwrap();
        cb
    }
}

pub struct Window {
    pub surface: WlSurface,
    pub shell_surface: WlShellSurface,
    pub webos: WlWebosShellSurface,
}

impl Dispatch<WlRegistry, GlobalListContents> for ClientState {
    fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlCallback, ()> for ClientState {
    fn event(state: &mut Self, _: &WlCallback, event: wayland_client::protocol::wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wayland_client::protocol::wl_callback::Event::Done { .. } = event {
            state.frames_done += 1;
        }
    }
}

impl Dispatch<WlBuffer, ()> for ClientState {
    fn event(state: &mut Self, b: &WlBuffer, event: wayland_client::protocol::wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wayland_client::protocol::wl_buffer::Event::Release = event {
            state.releases.push(b.id().protocol_id());
        }
    }
}

impl Dispatch<WlShellSurface, ()> for ClientState {
    fn event(state: &mut Self, s: &WlShellSurface, event: wl_shell_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_shell_surface::Event::Ping { serial } => s.pong(serial),
            wl_shell_surface::Event::Configure { width, height, .. } => state.configures.push((width, height)),
            _ => {}
        }
    }
}

impl Dispatch<WlWebosShellSurface, ()> for ClientState {
    fn event(state: &mut Self, s: &WlWebosShellSurface, event: wl_webos_shell_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_webos_shell_surface::Event::StateChanged { state: st } => {
                state.shell_states.push(st);
                state.states_of.push((s.id().protocol_id(), st));
            }
            wl_webos_shell_surface::Event::PositionChanged { x, y } => state.positions.push((x, y)),
            wl_webos_shell_surface::Event::Exposed { rectangles } => {
                let ints = rectangles.chunks_exact(4).map(|c| i32::from_ne_bytes([c[0], c[1], c[2], c[3]])).collect();
                state.exposed_of.push((s.id().protocol_id(), ints));
            }
            wl_webos_shell_surface::Event::Close => state.closes += 1,
            _ => {}
        }
    }
}

impl Dispatch<WlWebosExported, ()> for ClientState {
    fn event(state: &mut Self, _: &WlWebosExported, event: wl_webos_exported::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_webos_exported::Event::WindowIdAssigned { window_id, exported_type } = event {
            state.window_ids.push((window_id, exported_type));
        }
    }
}

impl Dispatch<WlWebosImported, ()> for ClientState {
    fn event(state: &mut Self, _: &WlWebosImported, event: wl_webos_imported::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_webos_imported::Event::PunchthroughAttached { context_id } => state.punchthrough_attached.push(context_id),
            wl_webos_imported::Event::DestinationRegionChanged { width, height } => state.destination_changed.push((width, height)),
            _ => {}
        }
    }
}

impl Dispatch<MaliBufferSharing, ()> for ClientState {
    fn event(state: &mut Self, _: &MaliBufferSharing, event: mali_buffer_sharing::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let mali_buffer_sharing::Event::AllocDevice { path } = event {
            state.alloc_devices.push(path);
        }
    }
}

impl Dispatch<WlKeyboard, ()> for ClientState {
    fn event(state: &mut Self, kb: &WlKeyboard, event: wayland_client::protocol::wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use wayland_client::protocol::wl_keyboard::Event;
        match event {
            Event::Enter { keys, .. } => {
                state.keyboard_enters += 1;
                state.enter_keys = keys.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            }
            Event::Key { key, state: st, .. } => {
                state.keys.push((key, st.into_result().map(|s| s as u32).unwrap_or(99)));
                state.keys_by_keyboard.push((kb.id().protocol_id(), key));
            }
            _ => {}
        }
    }
}

impl Dispatch<WlPointer, ()> for ClientState {
    fn event(state: &mut Self, _: &WlPointer, event: wayland_client::protocol::wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use wayland_client::protocol::wl_pointer::Event;
        match event {
            Event::Enter { surface_x, surface_y, .. } => state.pointer_enters.push((surface_x, surface_y)),
            Event::Leave { .. } => state.pointer_leaves += 1,
            Event::Motion { surface_x, surface_y, .. } => state.motions.push((surface_x, surface_y)),
            Event::Button { button, state: st, .. } => state.buttons.push((button, st.into_result().map(|s| s as u32).unwrap_or(99))),
            _ => {}
        }
    }
}

impl Dispatch<lumatv_wl_protocols::webos_input_manager::client::wl_webos_input_manager::WlWebosInputManager, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &lumatv_wl_protocols::webos_input_manager::client::wl_webos_input_manager::WlWebosInputManager,
        event: lumatv_wl_protocols::webos_input_manager::client::wl_webos_input_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let lumatv_wl_protocols::webos_input_manager::client::wl_webos_input_manager::Event::CursorVisibility { visibility, .. } = event {
            state.cursor_visibility.push(visibility);
        }
    }
}

impl Dispatch<WlSeat, ()> for ClientState {
    fn event(state: &mut Self, _: &WlSeat, event: wayland_client::protocol::wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use wayland_client::protocol::wl_seat::Event;
        match event {
            Event::Capabilities { capabilities } => state.seat_capabilities.push(capabilities.into_result().map(|c| c.bits()).unwrap_or(99)),
            Event::Name { name } => state.seat_names.push(name),
            _ => {}
        }
    }
}

impl Dispatch<lumatv_wl_protocols::webos_input_manager::client::wl_webos_seat::WlWebosSeat, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &lumatv_wl_protocols::webos_input_manager::client::wl_webos_seat::WlWebosSeat,
        event: lumatv_wl_protocols::webos_input_manager::client::wl_webos_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let lumatv_wl_protocols::webos_input_manager::client::wl_webos_seat::Event::Info { id, name, designator, capabilities } = event {
            state.webos_seat_infos.push((id, name, designator, capabilities));
        }
    }
}

impl Dispatch<lumatv_wl_protocols::starfish::client::wl_starfish_pointer::WlStarfishPointer, ()> for ClientState {
    fn event(
        state: &mut Self,
        _: &lumatv_wl_protocols::starfish::client::wl_starfish_pointer::WlStarfishPointer,
        event: lumatv_wl_protocols::starfish::client::wl_starfish_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let lumatv_wl_protocols::starfish::client::wl_starfish_pointer::Event::TimerValue { value } = event {
            state.starfish_timer_values.push(value);
        }
    }
}

delegate_noop!(ClientState: ignore wayland_client::protocol::wl_touch::WlTouch);
delegate_noop!(ClientState: ignore lumatv_wl_protocols::starfish::client::wl_starfish_output::WlStarfishOutput);
delegate_noop!(ClientState: ignore lumatv_wl_protocols::lumatv_subtitles_v1::client::lumatv_subtitle_manager_v1::LumatvSubtitleManagerV1);
delegate_noop!(ClientState: ignore lumatv_wl_protocols::lumatv_subtitles_v1::client::lumatv_subtitle_region_v1::LumatvSubtitleRegionV1);

delegate_noop!(ClientState: ignore WlCompositor);
delegate_noop!(ClientState: ignore WlShell);
delegate_noop!(ClientState: ignore WlShm);
delegate_noop!(ClientState: ignore WlShmPool);
delegate_noop!(ClientState: ignore WlSurface);
delegate_noop!(ClientState: ignore WlRegion);
delegate_noop!(ClientState: ignore WlSubcompositor);
delegate_noop!(ClientState: ignore WlSubsurface);
delegate_noop!(ClientState: ignore WlWebosShell);
delegate_noop!(ClientState: ignore WlWebosForeign);
delegate_noop!(ClientState: ignore WlWebosSurfaceGroupCompositor);
delegate_noop!(ClientState: ignore WlWebosSurfaceGroup);
delegate_noop!(ClientState: ignore WlWebosSurfaceGroupLayer);
delegate_noop!(ClientState: ignore TextModelFactory);
delegate_noop!(ClientState: ignore InputPanel);
delegate_noop!(ClientState: ignore InputPanelSurface);

impl Dispatch<TextModel, ()> for ClientState {
    fn event(state: &mut Self, _: &TextModel, event: text_model::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            text_model::Event::CommitString { text, .. } => state.ime.commits.push(text),
            text_model::Event::PreeditString { text, .. } => state.ime.preedits.push(text),
            text_model::Event::DeleteSurroundingText { index, length, .. } => state.ime.deletes.push((index, length)),
            text_model::Event::Keysym { sym, state: st, .. } => state.ime.keysyms.push((sym, st)),
            text_model::Event::Enter { .. } => state.ime.model_enters += 1,
            text_model::Event::Leave => state.ime.model_leaves += 1,
            text_model::Event::InputPanelState { state: s } => state.ime.panel_states.push(s),
            text_model::Event::InputPanelRect { x, y, width, height } => state.ime.panel_rects.push((x, y, width, height)),
            _ => {}
        }
    }
}

impl Dispatch<InputMethod, ()> for ClientState {
    fn event(state: &mut Self, _: &InputMethod, event: input_method::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            input_method::Event::Activate { id, .. } => {
                state.ime.activations += 1;
                state.ime.contexts.push(id);
            }
            input_method::Event::Deactivate { context } => state.ime.deactivated.push(context.id().protocol_id()),
            input_method::Event::ShowInputPanel { .. } => state.ime.shows += 1,
            input_method::Event::HideInputPanel { .. } => state.ime.hides += 1,
            _ => {}
        }
    }

    event_created_child!(ClientState, InputMethod, [0 => (InputMethodContext, ())]);
}

impl Dispatch<InputMethodContext, ()> for ClientState {
    fn event(state: &mut Self, _: &InputMethodContext, event: input_method_context::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let line = match event {
            input_method_context::Event::SurroundingText { text, cursor, anchor } => format!("surrounding_text {text:?} {cursor} {anchor}"),
            input_method_context::Event::Reset { serial } => format!("reset {serial}"),
            input_method_context::Event::ContentType { hint, purpose } => format!("content_type {hint} {purpose}"),
            input_method_context::Event::EnterKeyType { enter_key_type } => format!("enter_key_type {enter_key_type}"),
            input_method_context::Event::InvokeAction { button, index } => format!("invoke_action {button} {index}"),
            input_method_context::Event::Commit => "commit".to_string(),
            input_method_context::Event::MaxTextLength { length } => format!("max_text_length {length}"),
            input_method_context::Event::PlatformData { text } => format!("platform_data {text:?}"),
            _ => return,
        };
        state.ime.context_events.push(line);
    }
}

// --- Allocation counting on the server thread (the pattern of LumaShow's alloc_tests) ---

thread_local! {
    static ARMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
static ALLOCATIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct Counting;

fn count() {
    if ARMED.with(std::cell::Cell::get) {
        ALLOCATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

// SAFETY: every call is forwarded to the system allocator unchanged.
unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        count();
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        count();
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { std::alloc::System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `f` with the calling thread's allocations counted.
pub fn counted(f: impl FnOnce()) -> usize {
    ALLOCATIONS.store(0, std::sync::atomic::Ordering::Relaxed);
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
    ALLOCATIONS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The Rust shell's Home as the UI engine draws it (feature `ui`, Mesa's software GL): a ground of one colour under the windows, on a GL output of the
/// compositor's own, as LumaTV runs. Without the engine native Home is never drawn and the compositor's transitions wait for it.
#[cfg(feature = "ui")]
pub mod home_engine {
    use super::Server;
    use compositor_protocol::{Event, Fill, Props, Rgba, Shape};
    use lumashow_ui::{Scene, SceneOptions, Ui};
    use lumatv_compositor::render::egl::Platform;
    use lumatv_compositor::render::gl::{BufferPath, Gl};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::time::{Duration, Instant};

    const MESA: &str = "/usr/lib/x86_64-linux-gnu/libEGL.so.1";

    /// Whether Mesa is there to run the engine on; sets the software rasteriser for this process when it is.
    pub fn available() -> bool {
        if !Path::new(MESA).exists() {
            eprintln!("skipped: no Mesa at {MESA}");
            return false;
        }
        // SAFETY: nothing else in this process reads the environment while it is set.
        unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
        true
    }

    pub struct HomeEngine {
        /// The engine's scene: tests add layers to Home through it.
        pub ui: Ui,
        hangup: UnixStream,
    }

    impl Drop for HomeEngine {
        fn drop(&mut self) {
            let _ = self.hangup.shutdown(std::net::Shutdown::Both);
        }
    }

    /// A server whose output has the engine attached, with Home's ground (`ground`, RGB) drawn once; the server does not tick on its own.
    pub fn start(ground: [u8; 3]) -> (Server, HomeEngine) {
        let shm = std::env::temp_dir().join(format!("lumatv-home-engine-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(&shm).unwrap();
        let dir = shm.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let server = Server::start_with(Box::new(move || {
            let mut gl = Gl::new(MESA, Platform::Surfaceless, 1920, 1080, BufferPath::Own, None).unwrap();
            tx.send(gl.attach_ui(MESA, 1000, dir).unwrap()).unwrap();
            gl.ui().unwrap().finish_handshake().unwrap();
            Box::new(gl)
        }));
        let app_end = rx.recv().unwrap();
        let hangup = app_end.try_clone().unwrap();
        let (scene, _, events) = Scene::over_with(app_end, SceneOptions { shm_dir: Some(shm), ..Default::default() }).unwrap();
        let ui = Ui::new(scene);
        let fill = Shape { corner_radius: 0.0, fill: Fill::Solid(Rgba(ground[0], ground[1], ground[2], 255)), border_width: 0.0, border: Rgba(0, 0, 0, 0) };
        let layer = ui.scene.shape(1920, 1080, 0, fill, Props::at(0.0, 0.0));
        std::mem::forget(layer);
        let first = ui.try_commit().unwrap().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let event = events.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("a presented UI frame");
            ui.scene.observe(&event);
            if matches!(event, Event::Frame { tag, .. } if tag >= first) {
                break;
            }
        }
        server.ticking.store(false, std::sync::atomic::Ordering::SeqCst);
        (server, HomeEngine { ui, hangup })
    }
}
