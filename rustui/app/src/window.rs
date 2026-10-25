//! The window: a full-screen client of LG's compositor, the way `docs/research/m64/wl.c` makes one: a `wl_shell` toplevel with a
//! `wl_webos_shell_surface` (appId, window type `_WEBOS_WINDOW_TYPE_OVERLAY`, state fullscreen) over a `wl_egl_window` that the
//! 64-bit Mali blob draws into and presents through its own window path (`zwp_linux_dmabuf_v1`, buffers from the heap shim).
//!
//! The Wayland connection is `libwayland-client`'s, loaded at run time: it is the bundle's copy (`gfx64/`), which the blob links as
//! well, so the display pointer given to EGL is the one the blob's calls go to.

use crate::home::Key;
use crate::output::{Output, write_png};
use lumatv_render::Gles;
use lumatv_render::host::Engine;
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell::WlWebosShell;
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell_surface::{self, WlWebosShellSurface};
use std::ffi::{CString, c_void};
use std::io;
use std::os::fd::AsRawFd;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::wl_shell::WlShell;
use wayland_client::protocol::wl_shell_surface::{self, WlShellSurface};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
use wayland_protocols::xdg::shell::client::xdg_surface::{self, XdgSurface};
use wayland_protocols::xdg::shell::client::xdg_toplevel::{self, XdgToplevel};
use wayland_protocols::xdg::shell::client::xdg_wm_base::{self, XdgWmBase};

#[derive(Default)]
struct State {
    keys: Vec<Key>,
    raw_keys: Vec<u32>,
    closed: bool,
    states: Vec<u32>,
    seat_keyboard: Option<WlKeyboard>,
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
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
            wl_webos_shell_surface::Event::StateChanged { state: s } => state.states.push(s),
            wl_webos_shell_surface::Event::Close => state.closed = true,
            _ => {}
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(state: &mut Self, seat: &WlSeat, event: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(c) } = event {
            if c.contains(wl_seat::Capability::Keyboard) && state.seat_keyboard.is_none() {
                state.seat_keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}

impl Dispatch<WlKeyboard, ()> for State {
    fn event(state: &mut Self, _: &WlKeyboard, event: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_keyboard::Event::Key { key, state: WEnum::Value(wl_keyboard::KeyState::Pressed), .. } = event {
            state.raw_keys.push(key);
            // evdev codes: LG's compositor is expected to send them as the kernel numbers them
            let mapped = match key {
                103 => Some(Key::Up),
                108 => Some(Key::Down),
                105 => Some(Key::Left),
                106 => Some(Key::Right),
                28 | 0x160 => Some(Key::Ok),
                158 | 1 => Some(Key::Back),
                _ => None,
            };
            if let Some(k) = mapped {
                state.keys.push(k);
            }
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
            state.closed = true;
        }
    }
}

delegate_noop!(State: ignore WlCompositor);
delegate_noop!(State: ignore WlShell);
delegate_noop!(State: ignore WlSurface);
delegate_noop!(State: ignore WlWebosShell);

pub struct Window {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    _surface: WlSurface,
    _shell_surface: Option<WlShellSurface>,
    _webos: Option<WlWebosShellSurface>,
    _xdg: Option<(XdgSurface, XdgToplevel)>,
    _seat: Option<WlSeat>,
}

unsafe extern "C" {
    // resolved from the Mali blob at run time; declared only so the type is named
}

fn blob_symbol(name: &str) -> io::Result<*mut c_void> {
    let cname = CString::new(name).map_err(io::Error::other)?;
    // SAFETY: dlsym over the default scope, where the blob was loaded RTLD_GLOBAL by `open`.
    let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, cname.as_ptr()) };
    if p.is_null() { Err(io::Error::other(format!("{name} is not exported by the GL driver"))) } else { Ok(p) }
}

/// Connects, makes the window and the GL context on it.
pub fn open(width: u32, height: u32, libmali: &str, window_type: &str, want_keys: bool) -> io::Result<(Gles, Box<dyn Output>)> {
    let conn = Connection::connect_to_env().map_err(|e| io::Error::other(format!("wayland connect: {e}")))?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| io::Error::other(format!("{e}")))?;
    let qh = queue.handle();
    let mut state = State::default();
    let compositor: WlCompositor = globals.bind(&qh, 1..=4, ()).map_err(|e| io::Error::other(format!("wl_compositor: {e}")))?;
    // LG's compositor speaks wl_shell with wl_webos_shell; anything else (a host's Weston, for the plumbing) gets xdg_shell.
    let shell: Option<WlShell> = globals.bind(&qh, 1..=1, ()).ok();
    let webos_shell: Option<WlWebosShell> = globals.bind(&qh, 1..=2, ()).ok();
    let seat: Option<WlSeat> = if want_keys { globals.bind(&qh, 1..=5, ()).ok() } else { None };

    let surface = compositor.create_surface(&qh, ());
    let (mut shell_surface, mut webos, mut xdg) = (None, None, None);
    if let (Some(shell), Some(webos_shell)) = (&shell, &webos_shell) {
        let ss = shell.get_shell_surface(&surface, &qh, ());
        ss.set_title("LumaTV Rust UI".into());
        let w = webos_shell.get_shell_surface(&surface, &qh, ());
        w.set_property("appId".into(), "org.nphil.lumatv.rustui".into());
        w.set_property("_WEBOS_WINDOW_TYPE".into(), window_type.into());
        w.set_property("title".into(), "LumaTV Rust UI".into());
        ss.set_toplevel();
        w.set_state(3 /* fullscreen */);
        shell_surface = Some(ss);
        webos = Some(w);
    } else {
        let base: XdgWmBase =
            globals.bind(&qh, 1..=6, ()).map_err(|e| io::Error::other(format!("neither wl_shell with wl_webos_shell nor xdg_wm_base: {e}")))?;
        let xs = base.get_xdg_surface(&surface, &qh, ());
        let top = xs.get_toplevel(&qh, ());
        top.set_title("LumaTV Rust UI".into());
        top.set_app_id("org.nphil.lumatv.rustui".into());
        top.set_fullscreen(None);
        surface.commit();
        xdg = Some((xs, top));
        eprintln!("window: no wl_shell/wl_webos_shell; using xdg_shell (a host compositor)");
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
    Ok((gles, Box::new(Window { conn, queue, state, _surface: surface, _shell_surface: shell_surface, _webos: webos, _xdg: xdg, _seat: seat })))
}

impl Window {
    /// Reads what the socket has (waiting at most `timeout_ms`; -1 for ever, 0 for none) and dispatches it.
    fn pump(&mut self, timeout_ms: i32) -> io::Result<()> {
        self.queue.dispatch_pending(&mut self.state).map_err(|e| io::Error::other(format!("dispatch: {e}")))?;
        let _ = self.conn.flush();
        if let Some(guard) = self.conn.prepare_read() {
            let fd = guard.connection_fd().as_raw_fd();
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            // SAFETY: a poll of one descriptor.
            let n = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
            if n > 0 {
                let _ = guard.read();
            } else {
                drop(guard);
            }
        } else if timeout_ms != 0 {
            // events were queued by another reader meanwhile: dispatch them below
        }
        self.queue.dispatch_pending(&mut self.state).map_err(|e| io::Error::other(format!("dispatch: {e}")))?;
        if self.state.closed {
            return Err(io::Error::other("the compositor closed the window"));
        }
        Ok(())
    }
}

impl Output for Window {
    fn present(&mut self, engine: &mut Engine) -> io::Result<()> {
        engine.gles().swap_buffers()
    }

    fn wait(&mut self, deadline_ns: Option<u64>) -> io::Result<()> {
        let now = lumatv_render::now_ns();
        let ms = match deadline_ns {
            Some(d) if d > now => ((d - now) / 1_000_000).min(1000) as i32,
            Some(_) => 0,
            None => 50,
        };
        self.pump(ms)
    }

    fn dispatch(&mut self) -> io::Result<Vec<Key>> {
        self.pump(0)?;
        if !self.state.raw_keys.is_empty() {
            eprintln!("keys from the compositor: {:?}", std::mem::take(&mut self.state.raw_keys));
        }
        Ok(std::mem::take(&mut self.state.keys))
    }

    fn screenshot(&mut self, engine: &mut Engine, path: &str) -> io::Result<()> {
        // what the window's framebuffer holds after a swap is the driver's: this reads the last frame drawn, which on the Mali is
        // only the picture if nothing was swapped since
        let rows = engine.read_window();
        write_png(path, 1920, 1080, &rows)
    }
}
