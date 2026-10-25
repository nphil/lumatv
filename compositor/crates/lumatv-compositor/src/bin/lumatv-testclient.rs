//! `lumatv-testclient`: a shm client that behaves like the X2 probe (frame k cleared to R = 25k, G = 0x80,
//! B = 255 - 25k) through `wl_shell` and `wl_webos_shell`, for runs without a GPU: the host, or the set when the
//! question is the socket and the shell rather than the buffer import.
//!
//! Usage: lumatv-testclient [--frames 6] [--size WxH] [--type _WEBOS_WINDOW_TYPE_CARD] [--title x]

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_buffer::{self, WlBuffer};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_shell::WlShell;
use wayland_client::protocol::wl_shell_surface::{self, WlShellSurface};
use wayland_client::protocol::wl_shm::{Format, WlShm};
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};

use lumatv_wl_protocols::webos_shell::client::wl_webos_shell::WlWebosShell;
use lumatv_wl_protocols::webos_shell::client::wl_webos_shell_surface::{self, WlWebosShellSurface};

#[derive(Default)]
struct App {
    frame_done: bool,
    released: Vec<u32>,
    states: Vec<u32>,
    configures: Vec<(i32, i32)>,
}

impl Dispatch<WlRegistry, GlobalListContents> for App {
    fn event(_: &mut Self, _: &WlRegistry, _: wayland_client::protocol::wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlCallback, ()> for App {
    fn event(app: &mut Self, _: &WlCallback, event: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let wl_callback::Event::Done { .. } = event {
            app.frame_done = true;
        }
    }
}

impl Dispatch<WlBuffer, u32> for App {
    fn event(app: &mut Self, _: &WlBuffer, event: wl_buffer::Event, index: &u32, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_buffer::Event::Release = event {
            app.released.push(*index);
        }
    }
}

impl Dispatch<WlShellSurface, ()> for App {
    fn event(app: &mut Self, s: &WlShellSurface, event: wl_shell_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_shell_surface::Event::Ping { serial } => s.pong(serial),
            wl_shell_surface::Event::Configure { width, height, .. } => {
                println!("[cli] configure {width}x{height}");
                app.configures.push((width, height));
            }
            _ => {}
        }
    }
}

impl Dispatch<WlWebosShellSurface, ()> for App {
    fn event(app: &mut Self, _: &WlWebosShellSurface, event: wl_webos_shell_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_webos_shell_surface::Event::StateChanged { state } => {
                println!("[cli] state_changed {state}");
                app.states.push(state);
            }
            wl_webos_shell_surface::Event::PositionChanged { x, y } => println!("[cli] position_changed {x},{y}"),
            wl_webos_shell_surface::Event::Close => println!("[cli] close"),
            _ => {}
        }
    }
}

delegate_noop!(App: ignore WlCompositor);
delegate_noop!(App: ignore WlShell);
delegate_noop!(App: ignore WlShm);
delegate_noop!(App: ignore WlShmPool);
delegate_noop!(App: ignore WlSurface);
delegate_noop!(App: ignore WlWebosShell);

fn main() {
    let mut frames = 6u32;
    let (mut w, mut h) = (1920i32, 1080i32);
    let mut window_type = "_WEBOS_WINDOW_TYPE_CARD".to_string();
    let mut title = "lumatv test client".to_string();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--frames" => frames = args[i + 1].parse().unwrap(),
            "--size" => {
                let (a, b) = args[i + 1].split_once('x').unwrap();
                w = a.parse().unwrap();
                h = b.parse().unwrap();
            }
            "--type" => window_type = args[i + 1].clone(),
            "--title" => title = args[i + 1].clone(),
            o => {
                eprintln!("unknown option {o}");
                std::process::exit(2);
            }
        }
        i += 2;
    }
    let conn = Connection::connect_to_env().expect("WAYLAND_DISPLAY / XDG_RUNTIME_DIR point at the compositor");
    let (globals, mut queue) = registry_queue_init::<App>(&conn).unwrap();
    let qh = queue.handle();
    let mut app = App::default();
    globals.contents().with_list(|l| {
        for g in l {
            println!("[cli] global {:2} {:<36} v{}", g.name, g.interface, g.version);
        }
    });
    let compositor: WlCompositor = globals.bind(&qh, 1..=4, ()).expect("wl_compositor");
    let shell: WlShell = globals.bind(&qh, 1..=1, ()).expect("wl_shell");
    let shm: WlShm = globals.bind(&qh, 1..=1, ()).expect("wl_shm");
    let webos_shell: WlWebosShell = globals.bind(&qh, 1..=2, ()).expect("wl_webos_shell");

    let surface = compositor.create_surface(&qh, ());
    let shell_surface = shell.get_shell_surface(&surface, &qh, ());
    shell_surface.set_title(title.clone());
    shell_surface.set_toplevel();
    let webos = webos_shell.get_shell_surface(&surface, &qh, ());
    webos.set_property("_WEBOS_WINDOW_TYPE".into(), window_type);
    webos.set_property("appId".into(), "org.nphil.lumatv.testclient".into());
    webos.set_property("title".into(), title);
    queue.roundtrip(&mut app).unwrap();

    // Three buffers like the Mali swapchain; frame k is the X2 colour.
    let stride = w * 4;
    let size = stride * h;
    let mut buffers = Vec::new();
    for k in 0..3u32 {
        // SAFETY: a memfd sized below; mapped for the writes.
        let raw = unsafe { libc::memfd_create(c"lumatv-testclient".as_ptr(), libc::MFD_CLOEXEC) };
        assert!(raw >= 0, "memfd_create");
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let file = std::fs::File::from(fd.try_clone().unwrap());
        file.set_len(size as u64).unwrap();
        let pool = shm.create_pool(fd.as_fd(), size, &qh, ());
        let buffer = pool.create_buffer(0, w, h, stride, Format::Argb8888, &qh, k);
        pool.destroy();
        buffers.push((buffer, file));
    }
    let t0 = Instant::now();
    for k in 0..frames {
        let (buffer, file) = &buffers[(k % 3) as usize];
        let r = (25 * k).min(255) as u8;
        let b = 255u8.saturating_sub(r);
        let px = u32::from_le_bytes([b, 0x80, r, 0xff]); // memory order B G R A = ARGB8888 little-endian
        let mut bytes = Vec::with_capacity(size as usize);
        for _ in 0..(w * h) {
            bytes.extend_from_slice(&px.to_le_bytes());
        }
        std::io::Write::write_all(&mut &*file, &bytes).unwrap();
        use std::io::Seek;
        (&*file).seek(std::io::SeekFrom::Start(0)).unwrap();
        app.frame_done = false;
        surface.attach(Some(buffer), 0, 0);
        surface.damage(0, 0, i32::MAX, i32::MAX);
        surface.frame(&qh, ());
        surface.commit();
        conn.flush().unwrap();
        let ts = Instant::now();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !app.frame_done && Instant::now() < deadline {
            queue.blocking_dispatch(&mut app).unwrap();
        }
        println!("[cli] frame {k}: colour {r:02x}80{b:02x}ff, callback after {:.1} ms{}", ts.elapsed().as_secs_f64() * 1000.0, if app.frame_done { "" } else { " (TIMEOUT)" });
    }
    println!("[cli] {frames} frames in {:.1} ms; releases {:?}; states {:?}; configures {:?}", t0.elapsed().as_secs_f64() * 1000.0, app.released, app.states, app.configures);
    // The server may already have exited after its last frame (--frames); that is not a failure here.
    let _ = queue.roundtrip(&mut app);
    println!("[cli] done");
}
