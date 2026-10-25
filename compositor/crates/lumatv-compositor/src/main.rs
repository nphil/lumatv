//! `lumatv-compositor`: the process. Sockets, the 60 Hz tick and the control socket on one epoll loop.

use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use wayland_server::{Display, ListeningSocket};

use lumatv_compositor::config::{Config, RendererKind, USAGE};
use lumatv_compositor::output::Output;
use lumatv_compositor::render::gl::{BufferPath, Gl};
use lumatv_compositor::render::software::Software;
use lumatv_compositor::server::state::{ClientHandler, State};
use lumatv_compositor::{control, luna};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    init_logging();
    if args.first().map(String::as_str) == Some("ctl") {
        let mut runtime_dir = std::path::PathBuf::from("/tmp/lumatv");
        let mut words = &args[1..];
        if words.first().map(String::as_str) == Some("--runtime-dir") && words.len() >= 2 {
            runtime_dir = std::path::PathBuf::from(&words[1]);
            words = &words[2..];
        }
        match control::client(&runtime_dir, words) {
            Ok(line) => print!("{line}"),
            Err(e) => {
                eprintln!("ctl: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    let config = match Config::parse(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            if !e.starts_with("lumatv-compositor") {
                eprintln!("{USAGE}");
            }
            std::process::exit(2);
        }
    };
    if let Err(e) = run(config) {
        eprintln!("lumatv-compositor: {e}");
        std::process::exit(1);
    }
}

fn init_logging() {
    let level = std::env::var("LUMATV_LOG").ok().and_then(|v| v.parse().ok()).unwrap_or(log::LevelFilter::Info);
    log::set_boxed_logger(Box::new(StderrLogger)).ok();
    log::set_max_level(level);
}

/// Lines carry CLOCK_MONOTONIC seconds, the clock the X1 client, the page-flip events and the kernel stamp with,
/// so a client's frame gap lines up with what the compositor was doing at that moment.
struct StderrLogger;

impl log::Log for StderrLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        // SAFETY: a stack timespec for clock_gettime.
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        let t = ts.tv_sec as f64 + ts.tv_nsec as f64 / 1e9;
        let _ = writeln!(io::stderr().lock(), "[srv {t:10.3}] {} {}", record.level().as_str().to_ascii_lowercase(), record.args());
    }

    fn flush(&self) {}
}

fn run(config: Config) -> io::Result<()> {
    // A runtime directory of its own is private; LG's (/tmp/xdg, root:compositor 0770, made by the unit's
    // ExecStartPre) keeps its mode, since every client relies on the group bit to reach the socket.
    if !config.runtime_dir.is_dir() {
        std::fs::create_dir_all(&config.runtime_dir)?;
        std::fs::set_permissions(&config.runtime_dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    }
    // SAFETY: setting an environment variable at start-up, before any thread exists.
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", &config.runtime_dir) };
    if let Some(dir) = &config.capture_dir {
        std::fs::create_dir_all(dir)?;
    }

    let mut display: Display<State> = Display::new().map_err(io::Error::other)?;
    let dh = display.handle();
    let listener = ListeningSocket::bind(&config.socket).map_err(|e| io::Error::other(format!("bind {}: {e}", config.socket)))?;
    // Clients run as other users (WebAppMgr as wam, native apps in jails) and connect() needs write permission on
    // the socket file; the directory's mode is what limits who gets that far.
    std::fs::set_permissions(config.runtime_dir.join(&config.socket), std::os::unix::fs::PermissionsExt::from_mode(0o777))?;

    // The C libwayland display pointer, for eglBindWaylandDisplayWL; only the aarch64 build runs on that backend.
    #[cfg(target_arch = "aarch64")]
    let wl_display_ptr: Option<*mut std::ffi::c_void> = Some(dh.backend_handle().display_ptr() as *mut std::ffi::c_void);
    #[cfg(not(target_arch = "aarch64"))]
    let wl_display_ptr: Option<*mut std::ffi::c_void> = None;

    #[cfg(feature = "ui")]
    let mut ui_demo = None;
    #[allow(unused_mut)]
    let mut make_gl = || -> io::Result<Gl> {
        #[allow(unused_mut)]
        let mut gl = Gl::new(config.driver.as_deref().unwrap(), config.platform, config.width, config.height, config.buffers, wl_display_ptr)?;
        #[cfg(feature = "ui")]
        if config.ui_demo {
            let shm = config.runtime_dir.join("ui-shm");
            std::fs::create_dir_all(&shm)?;
            let app = gl.attach_ui(config.driver.as_deref().unwrap(), lumatv_compositor::ui::demo::SYSTEM_Z, shm.clone())?;
            ui_demo = Some(lumatv_compositor::ui::demo::Demo::with(app, &shm, lumatv_compositor::ui::demo::Options::from_env())?);
            gl.ui().unwrap().finish_handshake()?;
        }
        Ok(gl)
    };
    let output: Box<dyn Output> = match config.renderer {
        RendererKind::Software => Box::new(Software::new(config.width, config.height)),
        RendererKind::Gl => {
            let mut gl = make_gl()?;
            let warmed = gl.warm_up(None);
            log::info!("gl warm-up {:.1} ms", warmed.as_secs_f64() * 1000.0);
            Box::new(gl)
        }
        RendererKind::Kms => {
            let gl = make_gl()?;
            Box::new(lumatv_compositor::kms::Kms::open(&config.card, gl, config.width, config.height)?)
        }
    };
    let own_mali = config.renderer == RendererKind::Software || config.buffers == BufferPath::Own;
    State::create_globals(&dh, &config, own_mali);

    let luna: Box<dyn luna::Bridge> = match std::env::var("LUMATV_LUNA_BRIDGE") {
        Ok(path) => match luna::Pipe::connect(&path) {
            Ok(p) => Box::new(p),
            Err(e) => {
                log::warn!("luna bridge {path}: {e}; running without");
                Box::new(luna::Null::default())
            }
        },
        Err(_) => Box::new(luna::Null::default()),
    };
    let mut state = State::new(dh, config.clone(), output, luna);
    if state.luna.is_connected() {
        state.luna_register();
    }
    log::info!(
        "listening on {}/{} ({} renderer, buffers {:?}{}, {} globals incl. {})",
        config.runtime_dir.display(),
        config.socket,
        state.output.name(),
        config.buffers,
        if own_mali { ", own mali_buffer_sharing" } else { ", mali_buffer_sharing from the driver" },
        if config.starfish { "starfish" } else { "no starfish" },
        "webos shell/foreign/group/input"
    );

    let control_listener = if config.control {
        let path = control::socket_path(&config.runtime_dir);
        let _ = std::fs::remove_file(&path);
        let l = UnixListener::bind(&path)?;
        l.set_nonblocking(true)?;
        Some(l)
    } else {
        None
    };

    let timer = timerfd(config.tick_ns)?;
    let epoll = Epoll::new()?;
    const TAG_WL: u64 = 1;
    const TAG_LISTEN: u64 = 2;
    const TAG_TICK: u64 = 3;
    const TAG_CONTROL: u64 = 4;
    const TAG_OUTPUT: u64 = 5;
    const TAG_LUNA: u64 = 6;
    const TAG_CONTROL_CLIENT: u64 = 1 << 32;
    const TAG_INPUT: u64 = 1 << 33;
    const TAG_UI: u64 = 10;
    epoll.add(display.backend().poll_fd().as_raw_fd(), TAG_WL)?;
    epoll.add(listener.as_raw_fd(), TAG_LISTEN)?;
    epoll.add(timer.as_raw_fd(), TAG_TICK)?;
    if let Some(l) = &control_listener {
        epoll.add(l.as_raw_fd(), TAG_CONTROL)?;
    }
    if let Some(fd) = state.output.event_fd() {
        epoll.add(fd, TAG_OUTPUT)?;
    }
    if let Some(fd) = state.luna.fd() {
        epoll.add(fd, TAG_LUNA)?;
    }
    if let Some(fd) = state.output.ui_fd() {
        epoll.add(fd, TAG_UI)?;
    }
    let mut inputs = if config.input { lumatv_compositor::input::open_all() } else { Vec::new() };
    // One wl_seat per device, as LG's compositor has them: a device's keys go out on its own seat.
    let seats: Vec<lumatv_compositor::server::seat::SeatId> = inputs.iter().map(|d| state.add_seat(&d.name)).collect();
    for (i, d) in inputs.iter().enumerate() {
        epoll.add(d.fd(), TAG_INPUT | i as u64)?;
    }
    let client_events = Arc::new(Mutex::new(Vec::new()));
    let mut control_clients: Vec<ControlClient> = Vec::new();
    let mut next_control = 0u64;
    let mut deadline = config.seconds.map(|s| Instant::now() + Duration::from_secs(s));
    let mut events = vec![libc::epoll_event { events: 0, u64: 0 }; 16];
    // After --frames is reached the server stays up to 2 s more while a client is connected, so the client's own
    // teardown (eglTerminate, buffer destroys) is served rather than cut with a connection reset.
    let mut linger_until: Option<Instant> = None;
    // Whether the tick is a one-shot at the output's latch (true) or the fixed interval (false).
    let mut paced = false;

    loop {
        if state.should_exit {
            let lingering = config.exit_on_disconnect && !state.clients.is_empty();
            match (lingering, linger_until) {
                (false, _) => break,
                (true, None) => linger_until = Some(Instant::now() + Duration::from_secs(2)),
                (true, Some(t)) if Instant::now() >= t => break,
                _ => {}
            }
        }
        let now = Instant::now();
        let wake = [deadline, linger_until].into_iter().flatten().min();
        let timeout = match wake {
            Some(d) => d.saturating_duration_since(now).as_millis().clamp(1, 1000) as i32,
            None => 1000,
        };
        let n = epoll.wait(&mut events, timeout)?;
        let woke = Instant::now();
        let mut handled = 0u32;
        for ev in &events[..n] {
            let tag = ev.u64;
            handled |= 1
                << (if tag & TAG_INPUT != 0 {
                    7
                } else if tag & TAG_CONTROL_CLIENT != 0 {
                    8
                } else {
                    tag.min(10) as u32
                });
            match tag {
                TAG_WL => {
                    display.dispatch_clients(&mut state)?;
                }
                TAG_LISTEN => {
                    while let Some(stream) = listener.accept()? {
                        let client =
                            display.handle().insert_client(stream, Arc::new(ClientHandler { events: client_events.clone() })).map_err(io::Error::other)?;
                        state.client_connected(client.id());
                    }
                }
                TAG_TICK => {
                    let mut expirations = [0u8; 8];
                    // SAFETY: reading the 8-byte expiration count the timerfd owes.
                    unsafe { libc::read(timer.as_raw_fd(), expirations.as_mut_ptr() as *mut libc::c_void, 8) };
                    #[cfg(feature = "ui")]
                    if let Some(demo) = &mut ui_demo {
                        demo.tick(lumatv_render::now_ns())?;
                    }
                    state.tick();
                    state.poll_holds();
                    rearm(&timer, &state, &mut paced, config.tick_ns)?;
                }
                TAG_OUTPUT => {
                    // Page flips landed: the pacer learned their timestamps, a frame waiting behind the flip went
                    // out, and the landed frames' callbacks are due.
                    match state.output.dispatch_events() {
                        Ok(n) => state.frames_landed(n),
                        Err(e) => log::warn!("output events: {e}"),
                    }
                }
                TAG_LUNA => {
                    luna_poll(&mut state);
                }
                TAG_UI => {
                    // The UI app committed: taken in now; drawn now on a still screen, else at the tick.
                    if state.ui_wake() {
                        handled |= 1 << 9;
                        state.tick();
                        rearm(&timer, &state, &mut paced, config.tick_ns)?;
                    }
                }
                TAG_CONTROL => {
                    if let Some(l) = &control_listener {
                        while let Ok((s, _)) = l.accept() {
                            // Never blocking: a client that sends half a line or stops reading must not hold the loop.
                            s.set_nonblocking(true)?;
                            let tag = TAG_CONTROL_CLIENT | next_control;
                            next_control += 1;
                            epoll.add(s.as_raw_fd(), tag)?;
                            control_clients.push(ControlClient { tag, stream: s, line: Vec::new() });
                        }
                    }
                }
                t if t & TAG_INPUT != 0 => {
                    let i = (t & 0xffff_ffff) as usize;
                    if let Some(d) = inputs.get_mut(i) {
                        let from_network = d.from_network;
                        let mut evs: [lumatv_compositor::input::Event; 64] = [lumatv_compositor::input::Event::Key { code: 0, pressed: false }; 64];
                        let mut n = 0;
                        let _ = d.drain(|ev| {
                            if n < evs.len() {
                                evs[n] = ev;
                                n += 1;
                            }
                        });
                        for &ev in &evs[..n] {
                            match ev {
                                lumatv_compositor::input::Event::Key { code, pressed } => state.key_event(seats[i], code, pressed, from_network),
                                lumatv_compositor::input::Event::Motion { x, y } => {
                                    lumatv_compositor::server::seat::pointer_motion(&mut state, x, y);
                                }
                            }
                        }
                    }
                }
                t if t & TAG_CONTROL_CLIENT != 0 => {
                    if let Some(pos) = control_clients.iter().position(|c| c.tag == tag) {
                        let done = match control_clients[pos].read() {
                            Ok(Some(line)) => match control::handle(&mut state, line.trim()) {
                                control::Reply::Now(reply) => !control_clients[pos].answer(&reply),
                                // The job and the socket go to a thread; the loop is done with this client.
                                control::Reply::Later(job) => {
                                    let client = control_clients.remove(pos);
                                    epoll.del(client.stream.as_raw_fd());
                                    client.answer_later(job);
                                    continue;
                                }
                            },
                            Ok(None) => false,
                            Err(_) => true,
                        };
                        if done {
                            let client = control_clients.remove(pos);
                            epoll.del(client.stream.as_raw_fd());
                        }
                    }
                }
                _ => {}
            }
        }
        state.drain_client_events(&client_events);
        // A commit onto a still screen (KMS) is drawn now rather than at the next tick.
        if state.wants_frame_now() {
            handled |= 1 << 9;
            state.tick();
            rearm(&timer, &state, &mut paced, config.tick_ns)?;
        }
        display.flush_clients()?;
        // Anything that holds the loop for half a refresh delays every client's callbacks and commits with it; the
        // line says what ran, on the clock the clients' own logs use.
        let spent = woke.elapsed();
        if spent > Duration::from_millis(8) {
            const NAMES: [&str; 11] = ["?", "clients", "accept", "tick", "control", "flip", "luna", "input", "control-client", "draw-now", "ui"];
            let what: Vec<&str> = (0..NAMES.len()).filter(|b| handled & (1 << b) != 0).map(|b| NAMES[b]).collect();
            log::warn!("loop: {:.1} ms in one pass ({})", spent.as_secs_f64() * 1000.0, what.join(" "));
        }
        if let Some(d) = deadline
            && Instant::now() >= d
        {
            log::info!("deadline after {} s", config.seconds.unwrap_or(0));
            state.should_exit = true;
            deadline = None;
        }
    }
    display.flush_clients()?;
    state.pngs.finish();
    log::info!("summary: {}", state.summary());
    println!("summary: {}", state.summary());
    if let Some(l) = control_listener {
        drop(l);
        let _ = std::fs::remove_file(control::socket_path(&config.runtime_dir));
    }
    if state.check.bad > 0 {
        std::process::exit(3);
    }
    Ok(())
}

fn luna_poll(state: &mut State) {
    for m in state.luna.poll() {
        match m {
            luna::FromBridge::Ping { seq } => state.luna.send(luna::ToBridge::Pong { seq }),
            luna::FromBridge::Inbound(i) => state.luna_inbound(i),
            luna::FromBridge::Event(e) => state.luna_event(e),
        }
    }
}

/// One client of the control socket: a non-blocking stream and the bytes of the line it is sending.
struct ControlClient {
    tag: u64,
    stream: UnixStream,
    line: Vec<u8>,
}

impl ControlClient {
    /// Reads what has arrived: `Some(line)` once a whole line is in, `None` while it is not, `Err` when the client is
    /// gone (or sends more than a command's worth without a newline).
    fn read(&mut self) -> io::Result<Option<String>> {
        let mut chunk = [0u8; 512];
        loop {
            match io::Read::read(&mut self.stream, &mut chunk) {
                Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
                Ok(n) => self.line.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if let Some(end) = self.line.iter().position(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(&self.line[..end]).into_owned();
            self.line.drain(..=end);
            return Ok(Some(line));
        }
        if self.line.len() > 4096 {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        Ok(None)
    }

    /// Writes a reply line without waiting: a client that is not reading loses it (and is dropped). `false` when it is
    /// gone.
    fn answer(&mut self, reply: &str) -> bool {
        let mut bytes = reply.as_bytes().to_vec();
        bytes.push(b'\n');
        let mut sent = 0;
        while sent < bytes.len() {
            match self.stream.write(&bytes[sent..]) {
                Ok(0) => return false,
                Ok(n) => sent += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    if e.kind() != io::ErrorKind::WouldBlock {
                        log::warn!("control: {e}");
                    }
                    return false;
                }
            }
        }
        true
    }

    /// Runs `job` on a thread with the socket and writes its reply there (a capture's PNG).
    fn answer_later(mut self, job: Box<dyn FnOnce() -> String + Send + 'static>) {
        if let Err(e) = std::thread::Builder::new().name("control".into()).spawn(move || {
            let reply = job();
            let _ = self.stream.set_nonblocking(false);
            let _ = self.stream.write_all(reply.as_bytes()).and_then(|_| self.stream.write_all(b"\n"));
        }) {
            log::warn!("control: no thread: {e}");
        }
    }
}

/// After a frame: the output's latch as a one-shot while it paces itself, else the fixed interval (re-armed
/// only when the timer was last set as a one-shot, so the periodic timer keeps its phase).
fn rearm(timer: &OwnedFd, state: &State, paced: &mut bool, tick_ns: u64) -> io::Result<()> {
    match state.output.next_frame_at(Instant::now()) {
        Some(at) => {
            arm_once(timer, at.saturating_duration_since(Instant::now()).max(Duration::from_micros(500)))?;
            *paced = true;
        }
        None if *paced => {
            arm_periodic(timer, tick_ns)?;
            *paced = false;
        }
        None => {}
    }
    Ok(())
}

/// Re-arms the tick as a one-shot `delay` from now.
fn arm_once(timer: &OwnedFd, delay: Duration) -> io::Result<()> {
    // SAFETY: arming a timerfd with a stack itimerspec.
    unsafe {
        let value = libc::timespec { tv_sec: delay.as_secs() as libc::time_t, tv_nsec: delay.subsec_nanos() as libc::c_long };
        let spec = libc::itimerspec { it_interval: libc::timespec { tv_sec: 0, tv_nsec: 0 }, it_value: value };
        if libc::timerfd_settime(timer.as_raw_fd(), 0, &spec, std::ptr::null_mut()) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Arms the tick as a repeating interval starting one interval from now.
fn arm_periodic(timer: &OwnedFd, interval_ns: u64) -> io::Result<()> {
    // SAFETY: arming a timerfd with a stack itimerspec.
    unsafe {
        let ts = libc::timespec { tv_sec: (interval_ns / 1_000_000_000) as libc::time_t, tv_nsec: (interval_ns % 1_000_000_000) as libc::c_long };
        let spec = libc::itimerspec { it_interval: ts, it_value: ts };
        if libc::timerfd_settime(timer.as_raw_fd(), 0, &spec, std::ptr::null_mut()) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn timerfd(interval_ns: u64) -> io::Result<OwnedFd> {
    // SAFETY: creating a timerfd; the fd is owned from here on.
    let fd = unsafe { libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_NONBLOCK | libc::TFD_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd is a fresh, valid timerfd.
    let timer = unsafe { OwnedFd::from_raw_fd(fd) };
    arm_periodic(&timer, interval_ns)?;
    Ok(timer)
}

struct Epoll {
    fd: OwnedFd,
}

impl Epoll {
    fn new() -> io::Result<Epoll> {
        // SAFETY: epoll_create1 returns a new fd or -1.
        let fd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Epoll { fd: unsafe { OwnedFd::from_raw_fd(fd) } })
    }

    fn add(&self, fd: i32, tag: u64) -> io::Result<()> {
        let mut ev = libc::epoll_event { events: libc::EPOLLIN as u32, u64: tag };
        // SAFETY: registering a live fd with a stack event struct.
        if unsafe { libc::epoll_ctl(self.fd.as_raw_fd(), libc::EPOLL_CTL_ADD, fd, &mut ev) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn del(&self, fd: i32) {
        // SAFETY: removing an fd; failure (already closed) is harmless.
        unsafe { libc::epoll_ctl(self.fd.as_raw_fd(), libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut()) };
    }

    fn wait(&self, events: &mut [libc::epoll_event], timeout_ms: i32) -> io::Result<usize> {
        // SAFETY: the buffer length is passed with the pointer.
        let n = unsafe { libc::epoll_wait(self.fd.as_raw_fd(), events.as_mut_ptr(), events.len() as i32, timeout_ms) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                return Ok(0);
            }
            return Err(e);
        }
        Ok(n as usize)
    }
}
