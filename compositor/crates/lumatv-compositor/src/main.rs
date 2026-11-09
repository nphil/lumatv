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
use lumatv_compositor::{control, logsink, luna, mem, ring, sched, ticker, wake};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
    // Datagram logging never waits for the collector, including at shutdown.
    let result = run(config);
    log::logger().flush();
    if let Err(e) = result {
        log::error!("lumatv-compositor: {e}");
        std::process::exit(1);
    }
}

fn init_logging() {
    let level = std::env::var("LUMATV_LOG").ok().and_then(|v| v.parse().ok()).unwrap_or(log::LevelFilter::Info);
    log::set_boxed_logger(Box::new(logsink::Logger::start())).ok();
    log::set_max_level(level);
    if let Some(output) = lumatv_compositor::childlog::pipe("lumatv-compositor", "") {
        // Native libraries still write stderr; the bounded drain never waits for the collector.
        unsafe { libc::dup2(output.as_raw_fd(), libc::STDERR_FILENO); }
    }
    #[cfg(feature = "ui")]
    lumatv_render::set_log_sink(|message| log::info!("renderer: {message}")).expect("renderer log sink initialized once");
}

fn run(config: Config) -> io::Result<()> {
    // Memory first, before the display, the GL driver and the first thread exist: whatever the frame path allocates or touches from
    // here on is under it (`mem`: no mmap/brk/trim in steady state, the touched pages pinned).
    let refused = mem::tune_malloc();
    let grown = mem::pregrow(mem::PREGROW);
    // A runtime directory of its own is private; LG's (/tmp/xdg, root:compositor 0770, made by the unit's
    // ExecStartPre) keeps its mode, since every client relies on the group bit to reach the socket.
    if !config.runtime_dir.is_dir() {
        std::fs::create_dir_all(&config.runtime_dir)?;
        std::fs::set_permissions(&config.runtime_dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    }
    // SAFETY: setting an environment variable at start-up, before any thread exists.
    unsafe { std::env::set_var("XDG_RUNTIME_DIR", &config.runtime_dir) };
    // Open the nonblocking logging descriptor before entering the frame loop.
    init_logging();
    if !refused.is_empty() {
        log::warn!("mem: malloc refused {}", refused.join(", "));
    }
    log::info!("mem: heap grown {} MiB in {:.1} ms; malloc takes no mmap, no trim", mem::PREGROW >> 20, grown.as_secs_f64() * 1e3);
    log::info!("mem: {}", mem::lock(config.mlock));
    // The fixed diagnostic trace is budgeted separately from the central text log.
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
    // The Rust shell's engine (`shell_rust.rs`), attached before the loop so that KMS makes its UI pages: the client's end of its
    // socket is handed to the client when it starts.
    let mut rust_socket: Option<std::os::unix::net::UnixStream> = None;
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
        #[cfg(feature = "ui")]
        if !config.shell_rust.is_empty() {
            let shm = config.runtime_dir.join("ui-shm");
            std::fs::create_dir_all(&shm)?;
            match lumatv_compositor::shell_rust::clear_client_files(&shm) {
                Ok(0) => {}
                Ok(n) => log::info!("shell-rust: {n} file(s) an earlier client left in {} removed", shm.display()),
                Err(e) => log::warn!("shell-rust: {}: {e}", shm.display()),
            }
            rust_socket = Some(gl.attach_ui(config.driver.as_deref().unwrap(), lumatv_compositor::shell_rust::SYSTEM_Z, shm)?);
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
            let opts = lumatv_compositor::kms::Options {
                pages: lumatv_compositor::afbc::PageKind::wanted(config.afbc, config.page_alpha),
                page_alpha: config.page_alpha,
                afbc: config.afbc,
                pointer: config.pointer,
                pointer_alpha: config.pointer_alpha,
            };
            Box::new(lumatv_compositor::kms::Kms::open(&config.card, gl, config.width, config.height, scanout, opts)?)
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
        // a panel left dim by a run that ended is put back (`paneldim`)
        state.panel_recover();
    }
    install_term_handler();
    state.tap = scanout_tap;
    state.meta = scanout_meta;
    state.wl_display = wl_display_ptr;
    if config.scanout_probe {
        state.probe = Some(lumatv_compositor::scanout::probe::Probe::open(&config.card));
    }
    // `--refresh 120`: the output is asked once it exists; a refusal (no divider on this kernel) keeps the default, and says so.
    if config.refresh_hz != lumatv_compositor::refresh::DEFAULT_HZ {
        match state.refresh_apply(config.refresh_hz) {
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
    const TAG_SHELL_RUST: u64 = 12;
    const TAG_IM: u64 = 13;
    const TAG_INPUT_DIR: u64 = 14;
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
    if let Some(socket) = rust_socket.take() {
        state.shell_rust.adopt_socket(socket);
    }
    state.shell_rust_start();
    if let Some(error) = state.shell_rust_failure("native UI failed to start") {
        return Err(io::Error::other(error));
    }
    let mut rust_fd = state.shell_rust_fd();
    if let Some(fd) = rust_fd {
        epoll.add(fd, TAG_SHELL_RUST)?;
    }
    let mut inputs = lumatv_compositor::input::Inputs::new(&config.input_dir);
    if config.input {
        // lginput2 makes its virtual devices anew at every wake from standby: nodes made after the start are opened as they appear. The watch
        // comes first, so that a node made while the first look is under way is not missed.
        if let Some(fd) = inputs.watch() {
            epoll.add(fd, TAG_INPUT_DIR)?;
        }
        // One wl_seat per device, as LG's compositor has them: a device's keys go out on its own seat.
        let found = inputs.scan(monotonic_ns(), &mut |name| state.add_seat(name));
        for line in &found.lines {
            log::info!("{line}");
        }
        for line in &found.problems {
            log::warn!("{line}");
        }
        for &(i, fd) in &found.opened {
            epoll.add(fd, TAG_INPUT | i as u64)?;
        }
    }
    // LG's Input Manager queue, where the compositor owns the set's input: lginput2's handshake at each reconnect of the Magic Remote is
    // answered from the loop (it waits 2 s for each answer, then gives the remote up for that long), and it is told where the pointer stands.
    if config.input && config.im_queue {
        match lumatv_compositor::im_queue::ImQueue::open() {
            Ok(im) => {
                epoll.add(im.fd(), TAG_IM)?;
                state.journal(format!(
                    "im: reading /IM_MQ_WEBOS for lginput2's requests ({} stale message(s) discarded, lginput2 {} /IM_MQ_MRCU)",
                    im.stats.stale,
                    if im.registered() { "reads" } else { "has not made" }
                ));
                state.im = Some(im);
                let visible = state.seat.cursor_visible;
                state.im_cursor_visible(visible);
            }
            Err(e) => state.journal(format!("im: /IM_MQ_WEBOS does not open ({e}); lginput2's handshake goes unanswered")),
        }
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
    // Passes since the last snapshot and what woke them: a healthy loop makes 70 to 80 a second on idle Home. A descriptor that polls
    // ready for good makes it spin (round 16: 99.9 % of a core for an hour, every pass cheap); the journal names what woke it.
    let mut passes = 0u32;
    let mut passes_woke = 0u32;
    let mut spin_told_ns = 0u64;
    // When the last pass ended and what it handled, and the last flip stamp read: a flip the loop reached late is
    // told from them whether the loop was busy when it landed or woke late for it.
    let mut pass_end_ns = monotonic_ns();
    let mut pass_handled = 0u32;
    let mut last_flip_seen: Option<u64> = None;

    loop {
        if let Some(error) = state.fatal_error.take() {
            return Err(io::Error::other(error));
        }
        if let Some(error) = state.shell_rust_failure("native UI failed") {
            return Err(io::Error::other(error));
        }
        if TERM.load(std::sync::atomic::Ordering::Relaxed) && !state.should_exit {
            state.journal("exit: asked to stop (SIGTERM or SIGINT)".into());
            state.should_exit = true;
        }
        if state.should_exit {
            let lingering = config.exit_on_disconnect && !state.clients.is_empty();
            match (lingering, linger_until) {
                (false, _) => break,
                (true, None) => linger_until = Some(Instant::now() + Duration::from_secs(2)),
                (true, Some(t)) if Instant::now() >= t => break,
                _ => {}
            }
        }
        if state.shell_rust_fd() != rust_fd {
            if let Some(old) = rust_fd {
                epoll.del(old);
            }
            rust_fd = state.shell_rust_fd();
            if let Some(fd) = rust_fd {
                epoll.add(fd, TAG_SHELL_RUST)?;
            }
        }
        let now = Instant::now();
        let wake = [deadline, linger_until, state.nooperation_deadline()].into_iter().flatten().min();
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
            // A mouse or a game controller that is only watched for activity (`input.rs`) streams events while it is used: they must not
            // keep the loop out of its slow tick the way a key does.
            let watched = tag & TAG_INPUT != 0 && inputs.is_watch((tag & 0xffff_ffff) as usize);
            if tag != TAG_TICK && !watched {
                tick.calm.event(woke_ns, tag & TAG_INPUT != 0);
                // (what asks for frames at every refresh is a client, a key, the control socket: see `Calm::busy`. A page flip landing is not,
                // nor is the UI engine's commit, its report over the shell channel or a message of the luna bridge: what these ask for the
                // engine says (`Output::ui_due_in`, which is `None` while a frame is wanted now or an animation runs) and `Tick::rearm` obeys.
                // A Home with two cameras on it commits a still and reports it twice a second and the sky restates its paths every 0.4 s,
                // which with a quarter second of calm after each kept the loop at the panel's rate on a Home at rest: 124 ticks a second
                // for the 24 frames the sky and the wallpaper draw)
                if !matches!(tag, TAG_OUTPUT | TAG_UI | TAG_SHELL_RUST | TAG_LUNA) {
                    tick.calm.busy(woke_ns);
                }
                saw_event = true;
            }
            // before the UI is asked for a frame: a key ends the calm in which its commits wait for the next step
            state.output.ui_set_calm(tick.calm.is_calm_between_frames(woke_ns));
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
                    // The Rust shell's engine socket arrives with its handshake (shell_rust.rs): a commit then wakes the loop.
                    if state.ui_socket_new {
                        state.ui_socket_new = false;
                        if let Some(fd) = state.output.ui_fd() {
                            epoll.add(fd, TAG_UI)?;
                        }
                    }
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
                    // The bridge's end closed: its socket polls readable for good, so it leaves the loop (it spun a core otherwise).
                    if !state.luna.is_connected()
                        && let Some(fd) = state.luna.fd()
                    {
                        epoll.del(fd);
                        state.journal("luna: the bridge closed its socket; nothing more is read from it".into());
                    }
                    // What a message changed is what the tick waits for now, not what it was armed for before it.
                    tick.rearm(&state)?;
                }
                TAG_UI => {
                    // The UI app committed: taken in now; drawn now on a still screen, else at the tick.
                    let drew = state.ui_wake();
                    if drew {
                        handled |= 1 << 9;
                        state.tick();
                    }
                    // (a frame the commit wants and the screen could not take at once is the next tick's, which `rearm` puts at the next
                    // refresh while `ui_due_in` is `None`; a commit that wants none before the step the UI asked for, the sky restating its
                    // paths, leaves the loop asleep)
                    tick.rearm(&state)?;
                }
                TAG_SHELL_RUST => {
                    if state.shell_rust_poll() {
                        handled |= 1 << 9;
                    }
                    tick.rearm(&state)?;
                }
                TAG_IM => {
                    state.im_poll();
                }
                TAG_INPUT_DIR => inputs.watched(woke_ns),
                TAG_CONTROL => {
                    if let Some(l) = &control_listener {
                        while let Ok((s, _)) = l.accept() {
                            // Never blocking: a client that sends half a line or stops reading must not hold the loop.
                            s.set_nonblocking(true)?;
                            let tag = TAG_CONTROL_CLIENT | next_control;
                            next_control += 1;
                            epoll.add(s.as_raw_fd(), tag)?;
                            control_clients.push(ControlClient { tag, stream: s, line: Vec::new(), phone: false });
                        }
                    }
                }
                t if t & TAG_INPUT != 0 => {
                    let i = (t & 0xffff_ffff) as usize;
                    if let Some(d) = inputs.device(i) {
                        let from_network = d.from_network;
                        let mut evs: [lumatv_compositor::input::Event; 64] = [lumatv_compositor::input::Event::Key { code: 0, pressed: false }; 64];
                        let mut n = 0;
                        let read = d.drain(|ev| {
                            // A spun wheel is one turn to the pointer: its notches must not crowd keys out of the buffer (turns the other way are another turn).
                            if matches!(ev, lumatv_compositor::input::Event::Wheel { .. }) && n > 0 && evs[n - 1] == ev {
                                return;
                            }
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
                        // A removed device (a remote that went, a virtual one its maker closed) polls readable for good and every read
                        // fails: it leaves the loop, or the loop spins on it. Its seat stays, for the clients that bound it and for the
                        // device made in its place (`Inputs::scan`).
                        let gone = match read {
                            Err(e) => Some(e.to_string()),
                            Ok(()) if ev.events & (libc::EPOLLHUP | libc::EPOLLERR) as u32 != 0 => Some("hung up".to_string()),
                            Ok(()) => None,
                        };
                        let seat = inputs.seat(i);
                        for &ev in &evs[..n] {
                            match ev {
                                lumatv_compositor::input::Event::Key { code, pressed } => state.key_event(seat, code, pressed, from_network),
                                lumatv_compositor::input::Event::Motion { x, y } => {
                                    lumatv_compositor::server::seat::remote_motion(&mut state, x, y);
                                }
                                lumatv_compositor::input::Event::Wheel { down } => {
                                    lumatv_compositor::server::seat::wheel_turn(&mut state);
                                    state.shell_rust_wheel_direction(down);
                                }
                                lumatv_compositor::input::Event::Activity => {
                                    state.care_input();
                                    state.shell_rust_activity();
                                }
                            }
                        }
                        if let Some(why) = gone
                            && let Some(d) = inputs.lose(i)
                        {
                            epoll.del(d.fd());
                            state.journal(format!("input: {} ({}) is gone ({why}); its seat stays, nothing more is read from it", d.name, d.path));
                        }
                    }
                }
                t if t & TAG_CONTROL_CLIENT != 0 => {
                    if let Some(pos) = control_clients.iter().position(|c| c.tag == tag) {
                        let done = match control_clients[pos].read() {
                            Ok(Some(line)) => match control::handle_from(&mut state, &mut control_clients[pos].phone, line.trim()) {
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
        // Nodes made since the last look (the directory's watch fired, or two seconds went by): a device lginput2 made anew at a wake is
        // read from here on.
        if inputs.due(woke_ns) {
            let found = inputs.scan(woke_ns, &mut |name| state.add_seat(name));
            for line in found.lines.into_iter().chain(found.problems) {
                state.journal(line);
            }
            for &(i, fd) in &found.opened {
                if let Err(e) = epoll.add(fd, TAG_INPUT | i as u64) {
                    inputs.lose(i);
                    state.journal(format!("input: slot {i} cannot be waited on ({e}); left alone"));
                }
            }
        }
        state.drain_client_events(&client_events);
        state.nooperation_tick();
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
        state.stats.pass_us_max = state.stats.pass_us_max.max(spent.as_micros() as u64);
        state.stats.passes += 1;
        state.stats.pass_us_total += spent.as_micros() as u64;
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
            ring::note(format_args!(
                "D frame {} render {} us owed {} px in {} rects, ui {} draws {} layers ({})",
                drawn_seen,
                state.stats.render_us_last,
                state.stats.owed_px_last,
                state.stats.rects_last,
                state.stats.ui_draws_last,
                state.stats.ui_layers_last,
                handled_names(handled)
            ));
        }
        passes += 1;
        passes_woke |= handled;
        if pass_end_ns.saturating_sub(snapshot_at_ns) >= 1_000_000_000 {
            let secs = pass_end_ns.saturating_sub(snapshot_at_ns) as f64 / 1e9;
            snapshot_at_ns = pass_end_ns;
            ring::note(format_args!(
                "S ticks {} drawn {} held {} releases {} rss {} KiB passes {} ({}); {}",
                state.stats.ticks,
                state.stats.frames_drawn,
                state.stats.held_ticks,
                state.stats.releases,
                rss_kib(),
                passes,
                handled_names(passes_woke),
                state.output.stats_line()
            ));
            if passes as f64 > SPIN_PASSES_PER_SECOND * secs && pass_end_ns.saturating_sub(spin_told_ns) >= 10_000_000_000 {
                spin_told_ns = pass_end_ns;
                state.journal(format!("loop: {passes} passes in {secs:.1} s, woken by {}", handled_names(passes_woke)));
            }
            passes = 0;
            passes_woke = 0;
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
    // the panel goes back to the user's brightness before the process ends (`paneldim`); the bridge's answer is waited for, a moment at most
    state.panel_exit(Duration::from_millis(800));
    state.pngs.finish();
    state.probe_poll();
    if let Some(p) = &state.probe {
        log::info!("{}", p.summary());
        let _ = writeln!(io::stdout().lock(), "{}", p.summary());
    }
    log::info!("summary: {}", state.summary());
    let _ = writeln!(io::stdout().lock(), "summary: {}", state.summary());
    if let Some(l) = control_listener {
        drop(l);
        let _ = std::fs::remove_file(control::socket_path(&config.runtime_dir));
    }
    if state.check.bad > 0 {
        log::logger().flush();
        std::process::exit(3);
    }
    Ok(())
}

/// SIGTERM or SIGINT was received: the loop ends at its next pass (the idle tick is at most 100 ms away).
static TERM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_term(_: libc::c_int) {
    TERM.store(true, std::sync::atomic::Ordering::Relaxed);
    // A loop that is wedged never reaches the flag: SIGALRM (default action: end the process) comes in 3 s, so a TERM still ends the process
    // as it did before the handler existed.
    // SAFETY: `alarm` is async-signal-safe.
    unsafe { libc::alarm(3) };
}

/// Ends the compositor the clean way on SIGTERM and SIGINT, so that the panel dim's put back (`State::panel_exit`) runs; before, either one
/// ended the process on the spot.
fn install_term_handler() {
    let handler = on_term as *const () as libc::sighandler_t;
    // SAFETY: `on_term` only stores an atomic and calls `alarm`.
    unsafe {
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGINT, handler);
    }
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
    /// The helper's phone-remote plugin said a paired phone is in session on this connection (`ctl phone session on`).
    phone: bool,
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

/// Passes a second above which the loop is spinning; idle Home makes 70 to 80 (round 16).
const SPIN_PASSES_PER_SECOND: f64 = 1000.0;

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
