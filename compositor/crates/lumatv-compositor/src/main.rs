//! `lumatv-compositor`: the process. Sockets, the 60 Hz tick and the control socket on one epoll loop.

use std::io::{self, BufRead, BufReader, Write};
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
    log::set_boxed_logger(Box::new(StderrLogger { start: Instant::now() })).ok();
    log::set_max_level(level);
}

struct StderrLogger {
    start: Instant,
}

impl log::Log for StderrLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let t = self.start.elapsed().as_secs_f64();
        let _ = writeln!(io::stderr().lock(), "[srv {t:8.3}] {} {}", record.level().as_str().to_ascii_lowercase(), record.args());
    }

    fn flush(&self) {}
}

fn run(config: Config) -> io::Result<()> {
    std::fs::create_dir_all(&config.runtime_dir)?;
    std::fs::set_permissions(&config.runtime_dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    // SAFETY: setting an environment variable at start-up, before any thread exists.
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", &config.runtime_dir) };
    if let Some(dir) = &config.capture_dir {
        std::fs::create_dir_all(dir)?;
    }

    let mut display: Display<State> = Display::new().map_err(io::Error::other)?;
    let dh = display.handle();
    let listener = ListeningSocket::bind(&config.socket).map_err(|e| io::Error::other(format!("bind {}: {e}", config.socket)))?;

    // The C libwayland display pointer, for eglBindWaylandDisplayWL; only the aarch64 build runs on that backend.
    #[cfg(target_arch = "aarch64")]
    let wl_display_ptr: Option<*mut std::ffi::c_void> = Some(dh.backend_handle().display_ptr() as *mut std::ffi::c_void);
    #[cfg(not(target_arch = "aarch64"))]
    let wl_display_ptr: Option<*mut std::ffi::c_void> = None;

    let output: Box<dyn Output> = match config.renderer {
        RendererKind::Software => Box::new(Software::new(config.width, config.height)),
        RendererKind::Gl => Box::new(Gl::new(config.driver.as_deref().unwrap(), config.width, config.height, config.buffers, wl_display_ptr)?),
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
    const TAG_CONTROL_CLIENT: u64 = 1 << 32;
    epoll.add(display.backend().poll_fd().as_raw_fd(), TAG_WL)?;
    epoll.add(listener.as_raw_fd(), TAG_LISTEN)?;
    epoll.add(timer.as_raw_fd(), TAG_TICK)?;
    if let Some(l) = &control_listener {
        epoll.add(l.as_raw_fd(), TAG_CONTROL)?;
    }
    let client_events = Arc::new(Mutex::new(Vec::new()));
    let mut control_clients: Vec<(u64, BufReader<UnixStream>)> = Vec::new();
    let mut next_control = 0u64;
    let deadline = config.seconds.map(|s| Instant::now() + Duration::from_secs(s));
    let mut events = vec![libc::epoll_event { events: 0, u64: 0 }; 16];

    while !state.should_exit {
        let timeout = match deadline {
            Some(d) => d.saturating_duration_since(Instant::now()).as_millis().min(1000) as i32,
            None => 1000,
        };
        let n = epoll.wait(&mut events, timeout)?;
        for ev in &events[..n] {
            let tag = ev.u64;
            match tag {
                TAG_WL => {
                    display.dispatch_clients(&mut state)?;
                }
                TAG_LISTEN => {
                    while let Some(stream) = listener.accept()? {
                        display.handle().insert_client(stream, Arc::new(ClientHandler { events: client_events.clone() })).map_err(io::Error::other)?;
                    }
                }
                TAG_TICK => {
                    let mut expirations = [0u8; 8];
                    // SAFETY: reading the 8-byte expiration count the timerfd owes.
                    unsafe { libc::read(timer.as_raw_fd(), expirations.as_mut_ptr() as *mut libc::c_void, 8) };
                    state.tick();
                    for m in state.luna.poll() {
                        if let luna::FromBridge::Ping { seq } = m {
                            state.luna.send(luna::ToBridge::Pong { seq });
                        } else {
                            log::info!("luna: {m:?}");
                        }
                    }
                }
                TAG_CONTROL => {
                    if let Some(l) = &control_listener {
                        while let Ok((s, _)) = l.accept() {
                            s.set_nonblocking(false)?;
                            let tag = TAG_CONTROL_CLIENT | next_control;
                            next_control += 1;
                            epoll.add(s.as_raw_fd(), tag)?;
                            control_clients.push((tag, BufReader::new(s)));
                        }
                    }
                }
                t if t & TAG_CONTROL_CLIENT != 0 => {
                    if let Some(pos) = control_clients.iter().position(|(t, _)| *t == tag) {
                        let mut line = String::new();
                        let done = match control_clients[pos].1.read_line(&mut line) {
                            Ok(0) | Err(_) => true,
                            Ok(_) => {
                                let reply = control::handle(&mut state, line.trim());
                                let s = control_clients[pos].1.get_mut();
                                let _ = s.write_all(reply.as_bytes()).and_then(|_| s.write_all(b"\n"));
                                false
                            }
                        };
                        if done {
                            let (_, r) = control_clients.remove(pos);
                            epoll.del(r.get_ref().as_raw_fd());
                        }
                    }
                }
                _ => {}
            }
        }
        state.drain_client_events(&client_events);
        display.flush_clients()?;
        if let Some(d) = deadline
            && Instant::now() >= d
        {
            log::info!("deadline after {} s", config.seconds.unwrap_or(0));
            state.should_exit = true;
        }
    }
    display.flush_clients()?;
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

fn timerfd(interval_ns: u64) -> io::Result<OwnedFd> {
    // SAFETY: creating and arming a timerfd with a stack itimerspec.
    unsafe {
        let fd = libc::timerfd_create(libc::CLOCK_MONOTONIC, libc::TFD_NONBLOCK | libc::TFD_CLOEXEC);
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let ts = libc::timespec { tv_sec: (interval_ns / 1_000_000_000) as libc::time_t, tv_nsec: (interval_ns % 1_000_000_000) as libc::c_long };
        let spec = libc::itimerspec { it_interval: ts, it_value: ts };
        if libc::timerfd_settime(fd, 0, &spec, std::ptr::null_mut()) < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(OwnedFd::from_raw_fd(fd))
    }
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
