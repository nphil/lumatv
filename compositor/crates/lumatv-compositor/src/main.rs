//! `lumatv-compositor`: the process. Sockets, the 60 Hz tick and the control socket on one epoll loop.

use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use wayland_server::{Display, ListeningSocket};

use lumatv_compositor::config::{Config, RendererKind, USAGE};
use lumatv_compositor::output::{Output, monotonic_ns};
use lumatv_compositor::render::gl::{BufferPath, Gl};
use lumatv_compositor::render::software::Software;
use lumatv_compositor::server::state::{ClientHandler, State};
use lumatv_compositor::{control, luna, mem, ring, sched, ticker, wake};

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
        // Every line the compositor logs is also in the trace ring, in order with the loop's own events, for the crash bundle.
        ring::note(format_args!("{} {}", record.level().as_str().to_ascii_lowercase(), record.args()));
    }

    fn flush(&self) {}
}

fn run(config: Config) -> io::Result<()> {
    // Memory first, before the display, the GL driver and the first thread exist: whatever the frame path allocates or touches from
    // here on is under it (`mem`: no mmap/brk/trim in steady state, the touched pages pinned).
    let refused = mem::tune_malloc();
    if !refused.is_empty() {
        log::warn!("mem: malloc refused {}", refused.join(", "));
    }
    let grown = mem::pregrow(mem::PREGROW);
    log::info!("mem: heap grown {} MiB in {:.1} ms; malloc takes no mmap, no trim", mem::PREGROW >> 20, grown.as_secs_f64() * 1e3);
    log::info!("mem: {}", mem::lock(config.mlock));
    // A runtime directory of its own is private; LG's (/tmp/xdg, root:compositor 0770, made by the unit's
    // ExecStartPre) keeps its mode, since every client relies on the group bit to reach the socket.
    if !config.runtime_dir.is_dir() {
        std::fs::create_dir_all(&config.runtime_dir)?;
        std::fs::set_permissions(&config.runtime_dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    }
    // SAFETY: setting an environment variable at start-up, before any thread exists.
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", &config.runtime_dir) };
    // The trace ring, where the supervisor finds it after a crash (the runtime directory is tmpfs): opened once the directory exists.
    if let Some(path) = config.ring_path() {
        match ring::open(&path, ring::SIZE) {
            Ok(()) => {
                ring::install_panic_hook();
                log::info!("ring: {} ({} KiB of text)", path.display(), ring::SIZE >> 10);
            }
            Err(e) => log::warn!("ring: {}: {e}", path.display()),
        }
    }
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
    // The descriptors of the buffers the driver serves (`scanout::tap`): the tap is on while direct scan-out or its off-screen proof is asked for,
    // and not otherwise, so a compositor run without them has no more in its message path than before (`ctl scanout direct` puts it on later).
    let scanout_meta = lumatv_compositor::scanout::meta::Registry::default();
    let scanout_tap = if config.scanout.mode == lumatv_compositor::scanout::Mode::Direct || config.scanout_probe {
        lumatv_compositor::scanout::tap::Tap::install(wl_display_ptr, &scanout_meta)
    } else {
        lumatv_compositor::scanout::tap::Tap::off()
    };
    if config.scanout.mode == lumatv_compositor::scanout::Mode::Direct || config.scanout_probe {
        log::info!("scanout: switch {}, acquire {}, protocol tap {}", config.scanout.mode.name(), config.scanout.acquire.name(), scanout_tap.status().report());
    }

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
            let scanout = lumatv_compositor::scanout::Scanout::new(config.scanout.clone(), scanout_meta.clone());
            Box::new(lumatv_compositor::kms::Kms::open(&config.card, gl, config.width, config.height, scanout)?)
        }
    };
    let own_mali = config.renderer == RendererKind::Software || config.buffers == BufferPath::Own;
    State::create_globals(&dh, own_mali);

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
    state.tap = scanout_tap;
    state.meta = scanout_meta;
    state.wl_display = wl_display_ptr;
    if config.scanout_probe {
        state.probe = Some(lumatv_compositor::scanout::probe::Probe::open(&config.card));
    }
    // `--refresh 120`: the output is asked once it exists; a refusal (no divider on this kernel) keeps the default, and says so.
    if config.refresh_hz != lumatv_compositor::refresh::DEFAULT_HZ {
        match state.output.set_refresh(config.refresh_hz) {
            Ok(hz) => state.journal(format!("refresh: {hz} Hz from the command line")),
            Err(e) => {
                log::warn!("refresh {} Hz refused: {e}; staying at {}", config.refresh_hz, lumatv_compositor::refresh::DEFAULT_HZ);
                state.config.refresh_hz = lumatv_compositor::refresh::DEFAULT_HZ;
                state.config.tick_ns = lumatv_compositor::refresh::period_ns(lumatv_compositor::refresh::DEFAULT_HZ);
            }
        }
    }
    log::info!(
        "listening on {}/{} ({} renderer, buffers {:?}{}, LG's globals incl. webos shell/foreign/group/input and starfish)",
        config.runtime_dir.display(),
        config.socket,
        state.output.name(),
        config.buffers,
        if own_mali { ", own mali_buffer_sharing" } else { ", mali_buffer_sharing from the driver" },
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

    // The loop's thread alone: the class resets in every thread or process it creates (`sched::fifo`).
    if config.rt_priority > 0 {
        match sched::fifo(config.rt_priority) {
            Ok(()) => log::info!("loop thread: {}", sched::describe()),
            Err(e) => log::info!("loop thread: SCHED_FIFO {} refused ({e}); {}", config.rt_priority, sched::describe()),
        }
    }
    if config.rt_mali > 0 {
        for line in sched::mali_threads(Some(config.rt_mali)) {
            log::info!("mali thread: {line}");
        }
    }
    let mut tick = ticker::Tick::new(state.config.tick_ns)?;
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
    const TAG_SHELL: u64 = 11;
    epoll.add(display.backend().poll_fd().as_raw_fd(), TAG_WL)?;
    epoll.add(listener.as_raw_fd(), TAG_LISTEN)?;
    epoll.add(tick.raw_fd(), TAG_TICK)?;
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
    // The shell client (shell.rs): its UI channel's wake-up; started once the socket listens, so it finds the display.
    state.shell_start();
    if let Some(fd) = state.shell_fd() {
        epoll.add(fd, TAG_SHELL)?;
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
    // The one-second snapshot of the ring: when it was last written, and the frames the last pass found drawn.
    let mut snapshot_at_ns = monotonic_ns();
    let mut drawn_seen = 0u64;
    // When the last pass ended and what it handled, and the last flip stamp read: a flip the loop reached late is
    // told from them whether the loop was busy when it landed or woke late for it.
    let mut pass_end_ns = monotonic_ns();
    let mut pass_handled = 0u32;
    let mut last_flip_seen: Option<u64> = None;

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
        let woke_ns = monotonic_ns();
        // Landed flips first: their callbacks start the clients' next frames, and anything handled before them in the
        // pass (a tick's draw, a client's buffer import) would delay those by its own length.
        events[..n].sort_by_key(|ev| ev.u64 != TAG_OUTPUT);
        let mut handled = 0u32;
        let mut saw_event = false;
        for ev in &events[..n] {
            let tag = ev.u64;
            if tag != TAG_TICK {
                tick.calm.event(woke_ns, tag & TAG_INPUT != 0);
                saw_event = true;
            }
            handled |= 1
                << (if tag & TAG_INPUT != 0 {
                    7
                } else if tag & TAG_CONTROL_CLIENT != 0 {
                    8
                } else {
                    tag.min(11) as u32
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
                    let expirations = tick.read_expirations();
                    if let Some(late) = wake::tick_fired(woke_ns, pass_end_ns, expirations, tick.period_ns())
                        && late > 500_000
                    {
                        ring::note(format_args!("W tick +{} us", late / 1000));
                    }
                    #[cfg(feature = "ui")]
                    if let Some(demo) = &mut ui_demo {
                        demo.tick(lumatv_render::now_ns())?;
                    }
                    state.tick();
                    state.poll_holds();
                    tick.rearm(&state)?;
                }
                TAG_OUTPUT => {
                    // Page flips landed: the pacer learned their timestamps, a frame waiting behind the flip went
                    // out, and the landed frames' callbacks are sent. The tick moves to the pacer's latch at once: the
                    // first flip of a run is what teaches the pacer the panel's phase, and a fixed tick that fired
                    // before the next latch drew X1's second frame alone on the tenth gate.
                    state.land_flips();
                    tick.rearm(&state)?;
                }
                TAG_LUNA => {
                    luna_poll(&mut state);
                }
                TAG_UI => {
                    // The UI app committed: taken in now; drawn now on a still screen, else at the tick.
                    if state.ui_wake() {
                        handled |= 1 << 9;
                        state.tick();
                        tick.rearm(&state)?;
                    }
                }
                TAG_SHELL => {
                    if state.shell_poll() {
                        handled |= 1 << 9;
                        tick.rearm(&state)?;
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
                        if let Some(stamp) = d.take_stamp_ns() {
                            let late = wake::record(&wake::INPUT, stamp, woke_ns, pass_end_ns);
                            if late > 500_000 {
                                ring::note(format_args!("W input +{} us", late / 1000));
                            }
                        }
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
            tick.rearm(&state)?;
        }
        // An event ends the slowed tick: the timer goes back to the panel's rate before the loop sleeps, not at the next slow tick.
        if saw_event && tick.is_slowed() {
            tick.rearm(&state)?;
        }
        display.flush_clients()?;
        // Anything that holds the loop for half a refresh delays every client's callbacks and commits with it; the
        // line says what ran, on the clock the clients' own logs use.
        let spent = woke.elapsed();
        if spent > Duration::from_millis(2) {
            ring::note(format_args!("P {:.1} ms in one pass ({})", spent.as_secs_f64() * 1000.0, handled_names(handled)));
        }
        if spent > Duration::from_millis(8) {
            log::warn!("loop: {:.1} ms in one pass ({})", spent.as_secs_f64() * 1000.0, handled_names(handled));
        }
        // A flip the loop reached late (its callbacks went out late): whether it was busy when the flip landed, or
        // idle and late to wake for it (the CPU given to another task, or the event late in the kernel).
        if let Some(stamp) = state.output.last_flip_ns()
            && last_flip_seen != Some(stamp)
        {
            last_flip_seen = Some(stamp);
            let reach = woke_ns.saturating_sub(stamp);
            let late = wake::record(&wake::FLIP, stamp, woke_ns, pass_end_ns);
            ring::note(format_args!("F flip {:.3} reached +{} us{}", stamp as f64 / 1e9, reach / 1000, if late > 500_000 { " (late)" } else { "" }));
            if reach > 3_000_000 {
                if pass_end_ns <= stamp {
                    log::info!(
                        "loop: woke {:.1} ms after the flip's stamp, idle since {:.1} ms before it",
                        reach as f64 / 1e6,
                        stamp.saturating_sub(pass_end_ns) as f64 / 1e6
                    );
                } else {
                    log::info!(
                        "loop: woke {:.1} ms after the flip's stamp; the pass before ({}) ran until {:.1} ms after it",
                        reach as f64 / 1e6,
                        handled_names(pass_handled),
                        pass_end_ns.saturating_sub(stamp) as f64 / 1e6
                    );
                }
            }
        }
        pass_end_ns = monotonic_ns();
        pass_handled = handled;
        if state.stats.frames_drawn != drawn_seen {
            drawn_seen = state.stats.frames_drawn;
            ring::note(format_args!("D frame {} render {} us ({})", drawn_seen, state.stats.render_us_last, handled_names(handled)));
        }
        if pass_end_ns.saturating_sub(snapshot_at_ns) >= 1_000_000_000 {
            snapshot_at_ns = pass_end_ns;
            ring::note(format_args!(
                "S ticks {} drawn {} held {} releases {} rss {} KiB; {}",
                state.stats.ticks,
                state.stats.frames_drawn,
                state.stats.held_ticks,
                state.stats.releases,
                rss_kib(),
                state.output.stats_line()
            ));
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
    state.probe_poll();
    if let Some(p) = &state.probe {
        log::info!("{}", p.summary());
        println!("{}", p.summary());
    }
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

/// What a loop pass handled, by the `handled` bits, for its log lines.
fn handled_names(handled: u32) -> String {
    const NAMES: [&str; 12] = ["?", "clients", "accept", "tick", "control", "flip", "luna", "input", "control-client", "draw-now", "ui", "shell"];
    let what: Vec<&str> = (0..NAMES.len()).filter(|b| handled & (1 << b) != 0).map(|b| NAMES[b]).collect();
    what.join(" ")
}

/// The resident set in KiB, from `/proc/self/statm` (0 where it cannot be read).
fn rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1).and_then(|p| p.parse::<u64>().ok())).map_or(0, |pages| pages * 4)
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
