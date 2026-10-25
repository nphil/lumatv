//! An in-process compositor (software renderer, pure-Rust backend) on one end of a socketpair and a
//! `wayland-client` on the other. The server runs on its own thread and ticks every 4 ms; tests reach into its
//! `State` through [`Server::eval`].

#![allow(dead_code)]

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
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
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop};
use wayland_server::Display;

use lumatv_compositor::config::Config;
use lumatv_compositor::luna;
use lumatv_compositor::output::Output;
use lumatv_compositor::render::software::Software;
use lumatv_compositor::server::state::{ClientHandler, State};

use lumatv_wl_protocols::mali_buffer_sharing::client::mali_buffer_sharing::{self, MaliBufferSharing};
use lumatv_wl_protocols::webos_foreign::client::wl_webos_exported::{self, WlWebosExported};
use lumatv_wl_protocols::webos_foreign::client::wl_webos_foreign::WlWebosForeign;
use lumatv_wl_protocols::webos_foreign::client::wl_webos_imported::{self, WlWebosImported};
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell::WlWebosShell;
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell_surface::{self, WlWebosShellSurface};
use lumatv_wl_protocols::webos_surface_group::client::wl_webos_surface_group::WlWebosSurfaceGroup;
use lumatv_wl_protocols::webos_surface_group::client::wl_webos_surface_group_compositor::WlWebosSurfaceGroupCompositor;
use lumatv_wl_protocols::webos_surface_group::client::wl_webos_surface_group_layer::WlWebosSurfaceGroupLayer;

type Job = Box<dyn FnOnce(&mut State) + Send>;

pub struct Server {
    jobs: Sender<Job>,
    thread: Option<JoinHandle<()>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    /// While false the server dispatches but does not tick (frames are drawn only when something forces one).
    pub ticking: Arc<std::sync::atomic::AtomicBool>,
    pub client_stream: Option<UnixStream>,
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
        let (stop2, ticking2) = (stop.clone(), ticking.clone());
        let thread = std::thread::spawn(move || run_server(server_end, rx, stop2, ticking2, output));
        Server { jobs, thread: Some(thread), stop, ticking, client_stream: Some(client_end) }
    }

    /// Runs `f` on the server thread with its state and returns the result.
    pub fn eval<R: Send + 'static>(&self, f: impl FnOnce(&mut State) -> R + Send + 'static) -> R {
        let (tx, rx) = channel();
        self.jobs
            .send(Box::new(move |s| {
                let _ = tx.send(f(s));
            }))
            .unwrap();
        rx.recv_timeout(Duration::from_secs(5)).expect("server thread answered")
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

fn run_server(
    stream: UnixStream,
    jobs: Receiver<Job>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    ticking: Arc<std::sync::atomic::AtomicBool>,
    output: OutputFactory,
) {
    let mut display: Display<State> = Display::new().unwrap();
    let dh = display.handle();
    let config = Config { frames: None, ..Config::default() };
    State::create_globals(&dh, &config, true);
    let output = output();
    let mut state = State::new(dh, config, output, Box::new(luna::Null::default()));
    let events = Arc::new(Mutex::new(Vec::new()));
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
    pub positions: Vec<(i32, i32)>,
    pub closes: u32,
    pub window_ids: Vec<(String, u32)>,
    pub punchthrough_attached: Vec<String>,
    pub destination_changed: Vec<(u32, u32)>,
    pub alloc_devices: Vec<String>,
    pub keyboard_enters: u32,
    pub keys: Vec<(u32, u32)>,
    pub pointer_enters: Vec<(f64, f64)>,
    pub pointer_leaves: u32,
    pub motions: Vec<(f64, f64)>,
    pub buttons: Vec<(u32, u32)>,
    pub cursor_visibility: Vec<u32>,
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

    pub fn global_names(&self) -> Vec<String> {
        self.globals.contents().with_list(|l| l.iter().map(|g| g.interface.clone()).collect())
    }

    /// A card window: surface + wl_shell toplevel + webOS shell surface of the given type.
    pub fn card(&mut self, window_type: &str, title: &str) -> Window {
        let surface = self.compositor.create_surface(&self.qh, ());
        let shell_surface = self.shell.get_shell_surface(&surface, &self.qh, ());
        shell_surface.set_title(title.into());
        shell_surface.set_toplevel();
        let webos = self.webos_shell.get_shell_surface(&surface, &self.qh, ());
        webos.set_property("_WEBOS_WINDOW_TYPE".into(), window_type.into());
        webos.set_property("appId".into(), format!("org.test.{}", title.to_lowercase()));
        Window { surface, shell_surface, webos }
    }

    /// An shm buffer filled with one premultiplied ARGB value.
    pub fn shm_buffer(&mut self, w: i32, h: i32, argb: u32) -> WlBuffer {
        let stride = w * 4;
        let size = stride * h;
        // SAFETY: memfd for the test; sized and mapped below.
        let fd = unsafe { libc::memfd_create(c"test-shm".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(fd >= 0);
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let file = std::fs::File::from(fd.try_clone().unwrap());
        file.set_len(size as u64).unwrap();
        let mut bytes = Vec::with_capacity(size as usize);
        for _ in 0..(w * h) {
            bytes.extend_from_slice(&argb.to_le_bytes());
        }
        std::io::Write::write_all(&mut &file, &bytes).unwrap();
        let pool = self.shm.create_pool(fd.as_fd(), size, &self.qh, ());
        let buffer = pool.create_buffer(0, w, h, stride, Format::Argb8888, &self.qh, ());
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
    fn event(state: &mut Self, _: &WlWebosShellSurface, event: wl_webos_shell_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_webos_shell_surface::Event::StateChanged { state: st } => state.shell_states.push(st),
            wl_webos_shell_surface::Event::PositionChanged { x, y } => state.positions.push((x, y)),
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
    fn event(state: &mut Self, _: &WlKeyboard, event: wayland_client::protocol::wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use wayland_client::protocol::wl_keyboard::Event;
        match event {
            Event::Enter { .. } => state.keyboard_enters += 1,
            Event::Key { key, state: st, .. } => state.keys.push((key, st.into_result().map(|s| s as u32).unwrap_or(99))),
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

delegate_noop!(ClientState: ignore WlCompositor);
delegate_noop!(ClientState: ignore WlShell);
delegate_noop!(ClientState: ignore WlShm);
delegate_noop!(ClientState: ignore WlShmPool);
delegate_noop!(ClientState: ignore WlSurface);
delegate_noop!(ClientState: ignore WlRegion);
delegate_noop!(ClientState: ignore WlSubcompositor);
delegate_noop!(ClientState: ignore WlSubsurface);
delegate_noop!(ClientState: ignore WlSeat);
delegate_noop!(ClientState: ignore WlWebosShell);
delegate_noop!(ClientState: ignore WlWebosForeign);
delegate_noop!(ClientState: ignore WlWebosSurfaceGroupCompositor);
delegate_noop!(ClientState: ignore WlWebosSurfaceGroup);
delegate_noop!(ClientState: ignore WlWebosSurfaceGroupLayer);

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
