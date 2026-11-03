//! Apps that die mid-teardown, on the real `lumatv-compositor` binary (software renderer): PlxNative's close, as on the set in
//! round 16, where 25 crashed closes in 15 minutes came before the compositor's loop thread sat at 99.9 % of a core. Each client
//! binds what PlxNative binds, draws a card driven by its frame callbacks, destroys its surface with a frame on its way and its next
//! commit made, and drops the connection without another word. The compositor must leave nothing of it behind and go back to idle.

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_keyboard::WlKeyboard;
use wayland_client::protocol::wl_pointer::WlPointer;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shell::WlShell;
use wayland_client::protocol::wl_shell_surface::WlShellSurface;
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::protocol::wl_touch::WlTouch;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};

use lumatv_wl_protocols::starfish::client::wl_starfish_pointer::WlStarfishPointer;
use lumatv_wl_protocols::webos_input_manager::client::wl_webos_input_manager::WlWebosInputManager;
use lumatv_wl_protocols::webos_input_manager::client::wl_webos_seat::WlWebosSeat;
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell::WlWebosShell;
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell_surface::WlWebosShellSurface;

#[derive(Default)]
struct App {
    frames_done: u32,
}

impl Dispatch<WlRegistry, GlobalListContents> for App {
    fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlCallback, ()> for App {
    fn event(app: &mut Self, _: &WlCallback, event: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_callback::Event::Done { .. } = event {
            app.frames_done += 1;
        }
    }
}

delegate_noop!(App: ignore WlCompositor);
delegate_noop!(App: ignore WlShell);
delegate_noop!(App: ignore WlShellSurface);
delegate_noop!(App: ignore WlShm);
delegate_noop!(App: ignore WlShmPool);
delegate_noop!(App: ignore WlBuffer);
delegate_noop!(App: ignore WlSurface);
delegate_noop!(App: ignore WlSeat);
delegate_noop!(App: ignore WlKeyboard);
delegate_noop!(App: ignore WlPointer);
delegate_noop!(App: ignore WlTouch);
delegate_noop!(App: ignore WlWebosShell);
delegate_noop!(App: ignore WlWebosShellSurface);
delegate_noop!(App: ignore WlWebosInputManager);
delegate_noop!(App: ignore WlWebosSeat);
delegate_noop!(App: ignore WlStarfishPointer);

struct Compositor {
    child: Child,
    dir: PathBuf,
}

impl Compositor {
    fn start(name: &str) -> Compositor {
        let dir = Self::dir(name);
        let child = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor"))
            .args(["--runtime-dir", dir.to_str().unwrap(), "--renderer", "software", "--seconds", "120"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let c = Compositor { child, dir };
        c.wait_ready();
        c
    }

    /// A fresh runtime directory for a test's compositor.
    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumatv-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !(self.dir.join("lumatv-control").exists() && self.dir.join("wayland-lumatv").exists()) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn journal(&self) -> Vec<String> {
        let out = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor")).args(["ctl", "--runtime-dir", self.dir.to_str().unwrap(), "journal"]).output().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("journal json");
        v["lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap_or_default().to_string()).collect()
    }

    fn status(&self) -> serde_json::Value {
        let out = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor")).args(["ctl", "--runtime-dir", self.dir.to_str().unwrap(), "status"]).output().unwrap();
        serde_json::from_slice(&out.stdout).expect("status json")
    }

    /// CPU seconds the process has used (user and system).
    fn cpu_seconds(&self) -> f64 {
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", self.child.id())).unwrap();
        let f: Vec<&str> = stat.rsplit_once(')').unwrap().1.split_whitespace().collect();
        // SAFETY: plain sysconf.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
        (f[11].parse::<f64>().unwrap() + f[12].parse::<f64>().unwrap()) / hz
    }

    /// The share of one core the process uses over `over`.
    fn load(&self, over: Duration) -> f64 {
        let a = self.cpu_seconds();
        std::thread::sleep(over);
        (self.cpu_seconds() - a) / over.as_secs_f64()
    }
}

impl Drop for Compositor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn shm_buffer(shm: &WlShm, qh: &QueueHandle<App>, w: i32, h: i32) -> (WlShmPool, WlBuffer) {
    let size = w * h * 4;
    // SAFETY: memfd for the test, sized below.
    let fd = unsafe { libc::memfd_create(c"crash-loop".as_ptr(), libc::MFD_CLOEXEC) };
    assert!(fd >= 0);
    // SAFETY: the descriptor was just created and is owned here.
    let fd = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    std::fs::File::from(fd.try_clone().unwrap()).set_len(size as u64).unwrap();
    let pool = shm.create_pool(fd.as_fd(), size, qh, ());
    let buffer = pool.create_buffer(0, w, h, w * 4, Format::Argb8888, qh, ());
    (pool, buffer)
}

/// One PlxNative close: the globals it binds, a card that draws on its frame callbacks for `frames` frames, then the surface destroyed
/// with a frame on its way and the next commit made, and the connection dropped.
fn crashing_app(socket: &Path, frames: u32) {
    let conn = Connection::from_socket(UnixStream::connect(socket).unwrap()).unwrap();
    let (globals, mut queue) = registry_queue_init::<App>(&conn).unwrap();
    let qh = queue.handle();
    let mut app = App::default();
    let compositor: WlCompositor = globals.bind(&qh, 1..=4, ()).unwrap();
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
    let shell: WlShell = globals.bind(&qh, 1..=1, ()).unwrap();
    let webos_shell: WlWebosShell = globals.bind(&qh, 1..=1, ()).unwrap();
    let seat: WlSeat = globals.bind(&qh, 1..=4, ()).unwrap();
    let _keyboard = seat.get_keyboard(&qh, ());
    let _pointer = seat.get_pointer(&qh, ());
    let _touch = seat.get_touch(&qh, ());
    let input_manager: WlWebosInputManager = globals.bind(&qh, 1..=1, ()).unwrap();
    let _webos_seat = input_manager.get_webos_seat(&seat, &qh, ());
    let starfish: WlStarfishPointer = globals.bind(&qh, 1..=1, ()).unwrap();
    starfish.set_mrcu_standby_timer(5000);
    let surface = compositor.create_surface(&qh, ());
    let shell_surface = shell.get_shell_surface(&surface, &qh, ());
    shell_surface.set_toplevel();
    let webos = webos_shell.get_shell_surface(&surface, &qh, ());
    webos.set_property("_WEBOS_WINDOW_TYPE".into(), "_WEBOS_WINDOW_TYPE_CARD".into());
    webos.set_property("appId".into(), "com.beb.plxnative".into());
    let buffers = [shm_buffer(&shm, &qh, 64, 64), shm_buffer(&shm, &qh, 64, 64)];
    let mut drawn = 0;
    while drawn < frames {
        surface.attach(Some(&buffers[drawn as usize % 2].1), 0, 0);
        surface.damage(0, 0, 64, 64);
        let _cb = surface.frame(&qh, ());
        surface.commit();
        drawn += 1;
        let deadline = Instant::now() + Duration::from_secs(2);
        while app.frames_done < drawn && Instant::now() < deadline {
            queue.blocking_dispatch(&mut app).unwrap();
        }
    }
    // The close: one more frame is committed and the surface destroyed at once, with no wait for anything, and the process gone.
    surface.attach(Some(&buffers[drawn as usize % 2].1), 0, 0);
    surface.damage(0, 0, 64, 64);
    let _cb = surface.frame(&qh, ());
    surface.commit();
    let _pending = surface.frame(&qh, ());
    surface.destroy();
    conn.flush().unwrap();
}

#[test]
fn apps_that_die_mid_teardown_leave_nothing_behind_and_the_loop_goes_back_to_idle() {
    let c = Compositor::start("crash-loop");
    let socket = c.dir.join("wayland-lumatv");
    // A first close sets up what the compositor keeps for good (the seat's first keyboard and so on); what follows is measured.
    crashing_app(&socket, 5);
    std::thread::sleep(Duration::from_millis(100));
    let before = c.status();
    let idle_before = c.load(Duration::from_millis(500));
    for _ in 0..8 {
        crashing_app(&socket, 5);
    }
    std::thread::sleep(Duration::from_millis(100));
    let after = c.status();
    let idle_after = c.load(Duration::from_millis(500));
    assert_eq!(after["clients"], before["clients"], "every client is gone");
    assert_eq!(after["held"], before["held"], "nothing a client left is still held");
    assert!(
        idle_after < 0.10 && idle_after < idle_before + 0.05,
        "idle at {:.1} % of a core after the closes ({:.1} % before)",
        idle_after * 100.0,
        idle_before * 100.0
    );
}

/// The descriptors the loop waits on that can end under it: an input device that goes (a remote's node removed, a virtual device its
/// maker closed: every read fails and it polls readable for good) and the luna bridge's socket closed at the other end. Either kept
/// the loop thread at 100 % of a core; each now leaves the loop, with a journal line.
#[test]
fn a_device_that_goes_and_a_bridge_that_closes_leave_the_loop_idle() {
    let dir = Compositor::dir("hot-fds");
    let input = dir.join("input");
    std::fs::create_dir_all(&input).unwrap();
    // A FIFO stands in for the device: once its writer closes it polls hung up for good, as a removed evdev node does.
    let device = input.join("event0");
    let path = std::ffi::CString::new(device.to_str().unwrap()).unwrap();
    // SAFETY: a path the test owns.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let bridge = dir.join("bridge.sock");
    let listener = std::os::unix::net::UnixListener::bind(&bridge).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_lumatv-compositor"))
        .args(["--runtime-dir", dir.to_str().unwrap(), "--renderer", "software", "--seconds", "60", "--input", "--input-dir", input.to_str().unwrap()])
        .env("LUMATV_LUNA_BRIDGE", &bridge)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let c = Compositor { child, dir };
    let (bridge_end, _) = listener.accept().unwrap();
    c.wait_ready();
    assert!(c.load(Duration::from_millis(500)) < 0.10, "idle before");

    // A key comes from the device, then it goes.
    let mut writer = std::fs::OpenOptions::new().write(true).open(&device).unwrap();
    let mut event = [0u8; 24];
    event[16..18].copy_from_slice(&1u16.to_ne_bytes()); // EV_KEY
    event[18..20].copy_from_slice(&28u16.to_ne_bytes()); // KEY_ENTER
    std::io::Write::write_all(&mut writer, &event).unwrap();
    drop(writer);
    // The supervisor's end of the bridge closes.
    drop(bridge_end);
    drop(listener);
    std::thread::sleep(Duration::from_millis(100));
    let load = c.load(Duration::from_millis(500));
    assert!(load < 0.10, "the loop at {:.1} % of a core with the device gone and the bridge closed", load * 100.0);
    let journal = c.journal();
    assert!(journal.iter().any(|l| l.starts_with("input: ") && l.contains("event0) is gone")), "{journal:?}");
    assert!(journal.iter().any(|l| l == "luna: the bridge closed its socket; nothing more is read from it"), "{journal:?}");
}
