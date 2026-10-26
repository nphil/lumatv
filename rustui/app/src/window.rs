//! The window: a client of LG's compositor (or of LumaTV's, which speaks the same protocol), the way `docs/research/m64/wl.c` makes
//! one: a `wl_shell` toplevel with a `wl_webos_shell_surface` (appId, window type, access policy, state) over a `wl_egl_window` that the
//! 64-bit Mali blob draws into and presents through its own window path (`zwp_linux_dmabuf_v1`, buffers from the heap shim).
//!
//! The Wayland connection is `libwayland-client`'s, loaded at run time: it is the bundle's copy (`gfx64/`), which the blob links as
//! well, so the display pointer given to EGL is the one the blob's calls go to. A host's Weston has neither `wl_shell` nor
//! `wl_webos_shell`: the window is then an `xdg_shell` toplevel, for the plumbing.
//!
//! What the compositor says comes back as [`Event`]s: the state of the window (`state_changed`), a request to close, the keys.
//!
//! Keys: LG's compositor (`luna-surfacemanager` built with its multi-input support) has one `wl_seat` per input device: the default
//! seat, one for each device that has sent a key since boot (the remote, the network remote `kb.py` writes into, a keyboard), made
//! when the device's first key arrives, and one spare already advertised for the next device. A key goes out on the seat of the
//! device that sent it (`WebOSSurfaceItem::processKeyEvent`, `seatFor`), so a client bound to the first seat alone hears nothing
//! from the remote. Every seat is bound, the ones that appear later too, and each one's keyboard is asked for.

use lumatv_render::Gles;
use lumatv_wl_protocols::webos_foreign::client::wl_webos_exported::{self, WlWebosExported};
use lumatv_wl_protocols::webos_foreign::client::wl_webos_foreign::WlWebosForeign;
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell::WlWebosShell;
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell_surface::{self, WlWebosShellSurface};
use std::collections::VecDeque;
use std::ffi::{CString, c_void};
use std::io;
use std::os::fd::{AsRawFd, RawFd};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_shell::WlShell;
use wayland_client::protocol::wl_shell_surface::{self, WlShellSurface};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

/// What the app is as a window.
#[derive(Clone, Debug)]
pub struct Spec {
    pub app_id: String,
    pub title: String,
    /// `_WEBOS_WINDOW_TYPE_CARD` for an app, `_WEBOS_WINDOW_TYPE_OVERLAY` for a layer over one.
    pub window_type: String,
    /// `_WEBOS_ACCESS_POLICY_KEYS_BACK`: the window takes the Back key itself. Without it the compositor closes the window on Back.
    pub keys_back: bool,
    /// The state to ask for at once (`wl_webos_shell_surface.set_state`): 3 (fullscreen) for a window that shows. `None` for an app
    /// that was preloaded: it asks when SAM relaunches it, which is how SAM learns it is in front.
    pub state: Option<u32>,
    /// Bind the seats' keyboards.
    pub keys: bool,
    /// Export a video element on the surface (`wl_webos_foreign.export_element`, type 0) and place it over the whole window: the
    /// media pipeline then puts its picture under the window's pixels, where they are transparent (the GeForce NOW app). The id
    /// the compositor assigns is [`Window::video_window_id`].
    pub video: bool,
}

/// What the compositor said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// `wl_webos_shell_surface.state_changed`: 0 default, 1 minimized, 2 maximized, 3 fullscreen.
    State(u32),
    /// `wl_webos_shell_surface.close` or `xdg_toplevel.close`: the compositor asks the window to go.
    Close,
    /// `wl_keyboard.key`: the Linux input code.
    Key { code: u32, pressed: bool },
}

/// The newest `wl_seat` version asked for: 5 adds `release`, for a seat that goes away; LG's compositor serves 4 (Qt 5.12).
const SEAT_VERSION: u32 = 5;

/// One of the compositor's seats, bound for its keyboard.
struct Seat {
    /// The global's registry name, which the seat's and the keyboard's events carry as their data.
    name: u32,
    seat: WlSeat,
    keyboard: Option<WlKeyboard>,
}

#[derive(Default)]
struct State {
    events: VecDeque<Event>,
    /// `Spec::keys`: whether seats are bound at all.
    keys: bool,
    seats: Vec<Seat>,
    /// `wl_webos_exported.window_id_assigned`: the exported video element's id.
    video_window_id: Option<String>,
}

impl State {
    fn bind_seat(&mut self, registry: &WlRegistry, name: u32, version: u32, qh: &QueueHandle<Self>) {
        if self.seats.iter().any(|s| s.name == name) {
            return;
        }
        let seat: WlSeat = registry.bind(name, version.min(SEAT_VERSION), qh, name);
        eprintln!("window: wl_seat {name} bound at version {} ({} seats)", seat.version(), self.seats.len() + 1);
        self.seats.push(Seat { name, seat, keyboard: None });
    }

    fn drop_seat(&mut self, name: u32) {
        let Some(at) = self.seats.iter().position(|s| s.name == name) else { return };
        let seat = self.seats.remove(at);
        if let Some(kb) = seat.keyboard
            && kb.version() >= 3
        {
            kb.release();
        }
        if seat.seat.version() >= 5 {
            seat.seat.release();
        }
        eprintln!("window: wl_seat {name} went away ({} seats)", self.seats.len());
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(state: &mut Self, registry: &WlRegistry, event: wl_registry::Event, _: &GlobalListContents, _: &Connection, qh: &QueueHandle<Self>) {
        match event {
            wl_registry::Event::Global { name, interface, version } if state.keys && interface == "wl_seat" => state.bind_seat(registry, name, version, qh),
            wl_registry::Event::GlobalRemove { name } => state.drop_seat(name),
            _ => {}
        }
    }
}

impl Dispatch<WlShellSurface, ()> for State {
    fn event(_: &mut Self, s: &WlShellSurface, event: wl_shell_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_shell_surface::Event::Ping { serial } = event {
            s.pong(serial);
        }
    }
}

impl Dispatch<WlWebosShellSurface, ()> for State {
    fn event(state: &mut Self, _: &WlWebosShellSurface, event: wl_webos_shell_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_webos_shell_surface::Event::StateChanged { state: s } => state.events.push_back(Event::State(s)),
            wl_webos_shell_surface::Event::Close => state.events.push_back(Event::Close),
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, u32> for State {
    fn event(state: &mut Self, seat: &WlSeat, event: wl_seat::Event, name: &u32, _: &Connection, qh: &QueueHandle<Self>) {
        match event {
            wl_seat::Event::Capabilities { capabilities } => {
                // A capability bit the generated enum does not know must not hide the keyboard bit.
                let has_keyboard = match capabilities {
                    WEnum::Value(c) => c.contains(wl_seat::Capability::Keyboard),
                    WEnum::Unknown(bits) => bits & wl_seat::Capability::Keyboard.bits() != 0,
                };
                let Some(s) = state.seats.iter_mut().find(|s| s.name == *name) else { return };
                match (has_keyboard, s.keyboard.is_some()) {
                    (true, false) => s.keyboard = Some(seat.get_keyboard(qh, *name)),
                    (false, true) => {
                        if let Some(kb) = s.keyboard.take()
                            && kb.version() >= 3
                        {
                            kb.release();
                        }
                    }
                    _ => {}
                }
            }
            wl_seat::Event::Name { name: label } => eprintln!("window: wl_seat {name} is {label:?}"),
            _ => {}
        }
    }
}

impl Dispatch<WlKeyboard, u32> for State {
    fn event(state: &mut Self, _: &WlKeyboard, event: wl_keyboard::Event, seat: &u32, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            // The compositor sends Linux input codes as the kernel numbers them (the xkb keycode is this plus 8).
            wl_keyboard::Event::Key { key, state: WEnum::Value(s), .. } => {
                state.events.push_back(Event::Key { code: key, pressed: !matches!(s, wl_keyboard::KeyState::Released) });
            }
            // The keymap's descriptor is closed with the event: codes are what the app reads.
            wl_keyboard::Event::Keymap { format, size, .. } => eprintln!("window: seat {seat} keymap {format:?}, {size} bytes"),
            wl_keyboard::Event::Enter { .. } => eprintln!("window: keyboard focus on seat {seat}"),
            wl_keyboard::Event::Leave { .. } => eprintln!("window: keyboard focus on seat {seat} left"),
            _ => {}
        }
    }
}

impl Dispatch<XdgWmBase, ()> for State {
    fn event(_: &mut Self, base: &XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for State {
    fn event(_: &mut Self, s: &XdgSurface, event: xdg_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event {
            s.ack_configure(serial);
        }
    }
}

impl Dispatch<XdgToplevel, ()> for State {
    fn event(state: &mut Self, _: &XdgToplevel, event: xdg_toplevel::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_toplevel::Event::Close = event {
            state.events.push_back(Event::Close);
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
impl Dispatch<WlWebosExported, ()> for State {
    fn event(state: &mut Self, _: &WlWebosExported, event: wl_webos_exported::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_webos_exported::Event::WindowIdAssigned { window_id, exported_type } = event {
            eprintln!("window: video element exported as {window_id} (type {exported_type})");
            state.video_window_id = Some(window_id);
        }
    }
}

delegate_noop!(State: ignore WlShell);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore WlWebosShell);
delegate_noop!(State: ignore WlWebosForeign);
delegate_noop!(State: ignore wayland_client::protocol::wl_region::WlRegion);

pub struct Window {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    /// Kept so that seats which appear later are bound (`Dispatch<WlRegistry>`).
    _registry: WlRegistry,
    _surface: WlSurface,
    _shell_surface: Option<WlShellSurface>,
    webos: Option<WlWebosShellSurface>,
    _xdg: Option<(XdgSurface, XdgToplevel)>,
    _exported: Option<WlWebosExported>,
}

fn blob_symbol(name: &str) -> io::Result<*mut c_void> {
    let cname = CString::new(name).map_err(io::Error::other)?;
    // SAFETY: dlsym over the default scope, where the blob was loaded RTLD_GLOBAL by `open`.
    let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, cname.as_ptr()) };
    if p.is_null() { Err(io::Error::other(format!("{name} is not exported by the GL driver"))) } else { Ok(p) }
}

/// Connects, makes the window and the GL context on it.
pub fn open(width: u32, height: u32, libmali: &str, spec: &Spec) -> io::Result<(Gles, Window)> {
    let conn = Connection::connect_to_env().map_err(|e| io::Error::other(format!("wayland connect: {e}")))?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| io::Error::other(format!("{e}")))?;
    let qh = queue.handle();
    let mut state = State { keys: spec.keys, ..State::default() };
    let compositor: WlCompositor = globals.bind(&qh, 1..=4, ()).map_err(|e| io::Error::other(format!("wl_compositor: {e}")))?;
    // LG's compositor speaks wl_shell with wl_webos_shell; anything else (a host's Weston, for the plumbing) gets xdg_shell.
    let shell: Option<WlShell> = globals.bind(&qh, 1..=1, ()).ok();
    // LG's tables, which the generated client follows, have the interface at version 1 (`compositor/protocol/webos/webos-shell.xml`).
    let webos_shell: Option<WlWebosShell> = globals.bind(&qh, 1..=1, ()).ok();
    let registry = globals.registry().clone();
    if spec.keys {
        // Every seat, not the first: LG's compositor has one per input device (see the module's account).
        for g in globals.contents().clone_list() {
            if g.interface == "wl_seat" {
                state.bind_seat(&registry, g.name, g.version, &qh);
            }
        }
    }

    let surface = compositor.create_surface(&qh, ());
    let (mut shell_surface, mut webos, mut xdg) = (None, None, None);
    if let (Some(shell), Some(webos_shell)) = (&shell, &webos_shell) {
        let ss = shell.get_shell_surface(&surface, &qh, ());
        ss.set_title(spec.title.clone());
        let w = webos_shell.get_shell_surface(&surface, &qh, ());
        w.set_property("appId".into(), spec.app_id.clone());
        w.set_property("_WEBOS_WINDOW_TYPE".into(), spec.window_type.clone());
        w.set_property("title".into(), spec.title.clone());
        if spec.keys_back {
            w.set_property("_WEBOS_ACCESS_POLICY_KEYS_BACK".into(), "true".into());
        }
        ss.set_toplevel();
        if let Some(s) = spec.state {
            w.set_state(s);
        }
        shell_surface = Some(ss);
        webos = Some(w);
    } else {
        let base: XdgWmBase =
            globals.bind(&qh, 1..=6, ()).map_err(|e| io::Error::other(format!("neither wl_shell with wl_webos_shell nor xdg_wm_base: {e}")))?;
        let xs = base.get_xdg_surface(&surface, &qh, ());
        let top = xs.get_toplevel(&qh, ());
        top.set_title(spec.title.clone());
        top.set_app_id(spec.app_id.clone());
        top.set_fullscreen(None);
        surface.commit();
        xdg = Some((xs, top));
        eprintln!("window: no wl_shell/wl_webos_shell; using xdg_shell (a host compositor)");
    }
    // The video element, before the first commit (the order vplane/src/vpl_wl.c uses on the set): the whole window, source and
    // destination alike, so the picture fills the screen under the window's transparent pixels.
    let mut exported = None;
    if spec.video {
        match globals.bind::<WlWebosForeign, _, _>(&qh, 1..=1, ()) {
            Ok(foreign) => {
                let e = foreign.export_element(&surface, 0, &qh, ());
                let src = compositor.create_region(&qh, ());
                src.add(0, 0, width as i32, height as i32);
                let dst = compositor.create_region(&qh, ());
                dst.add(0, 0, width as i32, height as i32);
                e.set_exported_window(&src, &dst);
                src.destroy();
                dst.destroy();
                exported = Some(e);
            }
            Err(e) => eprintln!("window: no wl_webos_foreign ({e}); the video has no element to show through"),
        }
    }
    queue.roundtrip(&mut state).map_err(|e| io::Error::other(format!("roundtrip: {e}")))?;

    // The blob first: its symbols (wl_egl_window_create, EGL, GLES) resolve against it from here.
    let path = CString::new(libmali).map_err(io::Error::other)?;
    // SAFETY: dlopen of the driver by path.
    let handle = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) };
    if handle.is_null() {
        return Err(io::Error::other(format!("cannot load {libmali}")));
    }
    let create = blob_symbol("wl_egl_window_create").or_else(|e| {
        // a driver that does not export it (Mesa's EGL does not) has it in libwayland-egl
        let lib = CString::new("libwayland-egl.so.1").map_err(io::Error::other)?;
        // SAFETY: dlopen of the system library by name.
        if unsafe { libc::dlopen(lib.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL) }.is_null() {
            return Err(e);
        }
        blob_symbol("wl_egl_window_create")
    })?;
    // SAFETY: wl_egl_window_create(struct wl_surface *, int, int) -> struct wl_egl_window *
    let create: unsafe extern "C" fn(*mut c_void, i32, i32) -> *mut c_void = unsafe { std::mem::transmute(create) };
    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let surface_ptr = surface.id().as_ptr() as *mut c_void;
    // SAFETY: both pointers are live proxies of the connection this window keeps for its whole life.
    let egl_window = unsafe { create(surface_ptr, width as i32, height as i32) };
    if egl_window.is_null() {
        return Err(io::Error::other("wl_egl_window_create failed"));
    }
    // SAFETY: the display, the egl window and the driver outlive the Gles (leaked with the process).
    let gles = unsafe { Gles::window(&path, display_ptr, egl_window)? };
    gles.set_swap_interval(1);
    let _ = queue.dispatch_pending(&mut state);
    let _ = conn.flush();
    Ok((gles, Window { conn, queue, state, _registry: registry, _surface: surface, _shell_surface: shell_surface, webos, _xdg: xdg, _exported: exported }))
}

impl Window {
    /// The exported video element's id (`_Window_Id_N`), once the compositor has assigned it.
    pub fn video_window_id(&self) -> Option<&str> {
        self.state.video_window_id.as_deref()
    }

    /// Reads what the connection has (waiting at most `timeout_ms`; -1 for ever, 0 for none) and dispatches it. `extra` is another
    /// descriptor to wake for; the answer says whether it is readable.
    pub fn wait(&mut self, timeout_ms: i32, extra: Option<RawFd>) -> io::Result<bool> {
        self.queue.dispatch_pending(&mut self.state).map_err(|e| io::Error::other(format!("dispatch: {e}")))?;
        let _ = self.conn.flush();
        let mut extra_ready = false;
        if let Some(guard) = self.conn.prepare_read() {
            let mut fds = [
                libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 },
                libc::pollfd { fd: extra.unwrap_or(-1), events: libc::POLLIN, revents: 0 },
            ];
            // SAFETY: a poll of two descriptors (a negative one is skipped).
            let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout_ms) };
            if n > 0 && fds[0].revents != 0 {
                let _ = guard.read();
            } else {
                drop(guard);
            }
            extra_ready = n > 0 && fds[1].revents != 0;
        }
        // Without a read guard another reader (EGL's own dispatch) has events for this queue already: dispatch them below.
        self.queue.dispatch_pending(&mut self.state).map_err(|e| io::Error::other(format!("dispatch: {e}")))?;
        Ok(extra_ready)
    }

    /// What the compositor has said since the last call.
    pub fn events(&mut self) -> Vec<Event> {
        self.state.events.drain(..).collect()
    }

    /// How many of the compositor's seats are bound, and how many of them gave a keyboard.
    pub fn seats(&self) -> (usize, usize) {
        (self.state.seats.len(), self.state.seats.iter().filter(|s| s.keyboard.is_some()).count())
    }

    /// Asks the compositor for a window state (`wl_webos_shell_surface.set_state`). It answers with `state_changed`.
    pub fn request_state(&mut self, state: u32) {
        if let Some(w) = &self.webos {
            w.set_state(state);
        }
        let _ = self.conn.flush();
    }
}
