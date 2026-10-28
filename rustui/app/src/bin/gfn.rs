//! `lumatv-gfn-app`: the GeForce NOW app's 64-bit UI, a card window of LG's compositor (and, unchanged, of LumaTV's) drawn with
//! LumaShow's toolkit over the video plane the feeder fills. It starts `opennow-core` (which starts the stream engine beside it),
//! speaks the core protocol (`gfn/core.rs`), runs the rules (`gfn/app.rs`) and draws the screens (`gfn/ui.rs`). The feeder, the
//! app's `main`, starts this through `gfx64/launch.sh`. `docs/research/gfn-app-result.md` has the account.
//!
//!   lumatv-gfn-app [--app-id ID] [--assets DIR] [--shm DIR] [--libmali PATH] [--core RUNNER] [--core-arg ARG]... [--engine PATH]
//!                  [--data DIR] [--pad-side PATH] [--feeder-socket PATH] [--channel-fd N] [--hidden]
//!
//! `--channel-fd` is the feeder's socket pair (`lifecycle.rs`): SAM's relaunch and close arrive on it, the window's state and the end go
//! back. `--hidden` is a preload: nothing is drawn until the compositor brings the window forward.
//!
//! The pad's side channel (`--pad-side`, a datagram socket the engine writes to while the stream's input is paused) drives the
//! overlay with the same controller as the game.
use lumatv_render::host::{Engine, HostConfig};
use lumatv_render::{Demand, Gles, now_ns};
use lumatv_rustui::card::{FULLSCREEN, MINIMIZED, key_of};
use lumatv_rustui::gfn::app::{Action, App, Input, PadKey, PadLatency};
use lumatv_rustui::gfn::core::{CoreClient, CoreMsg};
use lumatv_rustui::gfn::layout::{H, W};
use lumatv_rustui::gfn::prefs::Prefs;
use lumatv_rustui::gfn::ui::GfnUi;
use lumatv_rustui::lifecycle::{Channel, FromApp, ToApp};
use lumatv_rustui::look::Mode;
use lumatv_rustui::window::{self, Event, Spec};
use serde_json::Value;
use std::io;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use ui::{SceneOptions, Ui};

struct Options {
    app_id: String,
    assets: PathBuf,
    shm: PathBuf,
    libmali: Option<String>,
    core: PathBuf,
    core_args: Vec<String>,
    engine: PathBuf,
    data: PathBuf,
    pad_side: PathBuf,
    feeder_socket: PathBuf,
    window_id_file: PathBuf,
    /// The text scale (1.0; Lucent's 200 % check is 2.0) and the material (`dark`, `light`; absent: the wallpaper's brightness decides).
    text_scale: f32,
    mode: Option<Mode>,
    prefs: Option<PathBuf>,
    /// The feeder's channel (descriptor number) for SAM's relaunch and close, and whether SAM started the app as a preload.
    channel_fd: Option<i32>,
    hidden: bool,
}

fn parse() -> Result<Options, String> {
    let here = std::env::current_exe().ok().and_then(|p| p.parent().and_then(|b| b.parent()).map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("."));
    let mut o = Options {
        app_id: "org.nphil.lumatv.gfn".into(),
        assets: here.join("assets"),
        shm: std::env::temp_dir().join("lumatv-gfn-shm"),
        libmali: std::env::var("LUMATV_GFX_LIB").ok(),
        core: here.join("run64.sh"),
        core_args: vec!["opennow-core".into()],
        engine: here.join("bin64/opennow-streamer"),
        data: PathBuf::from("/var/lib/lumatv-gfn"),
        pad_side: PathBuf::from("/tmp/lumatv-gfn/pad.sock"),
        feeder_socket: PathBuf::from("/tmp/lumatv-gfn/feed.sock"),
        window_id_file: PathBuf::from("/tmp/lumatv-gfn/window-id"),
        text_scale: 1.0,
        mode: None,
        prefs: None,
        channel_fd: None,
        hidden: false,
    };
    let mut args = std::env::args().skip(1);
    let mut core_args_given = false;
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--app-id" => o.app_id = value("--app-id")?,
            "--assets" => o.assets = PathBuf::from(value("--assets")?),
            "--shm" => o.shm = PathBuf::from(value("--shm")?),
            "--libmali" => o.libmali = Some(value("--libmali")?),
            "--core" => o.core = PathBuf::from(value("--core")?),
            "--core-arg" => {
                if !core_args_given {
                    o.core_args.clear();
                    core_args_given = true;
                }
                o.core_args.push(value("--core-arg")?);
            }
            "--engine" => o.engine = PathBuf::from(value("--engine")?),
            "--data" => o.data = PathBuf::from(value("--data")?),
            "--pad-side" => o.pad_side = PathBuf::from(value("--pad-side")?),
            "--feeder-socket" => o.feeder_socket = PathBuf::from(value("--feeder-socket")?),
            "--window-id-file" => o.window_id_file = PathBuf::from(value("--window-id-file")?),
            "--text-scale" => o.text_scale = value("--text-scale")?.parse::<f32>().map_err(|e| format!("--text-scale: {e}"))?,
            "--mode" => {
                o.mode = match value("--mode")?.as_str() {
                    "dark" => Some(Mode::Dark),
                    "light" => Some(Mode::Light),
                    "auto" => None,
                    other => return Err(format!("--mode is dark, light or auto, not {other}")),
                }
            }
            "--prefs" => o.prefs = Some(PathBuf::from(value("--prefs")?)),
            "--channel-fd" => o.channel_fd = Some(value("--channel-fd")?.parse().map_err(|e| format!("--channel-fd: {e}"))?),
            "--hidden" => o.hidden = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

fn log(what: &str) {
    eprintln!("gfn[{:.3}]: {what}", now_ns() as f64 / 1e9);
}

extern "C" fn on_term(_: libc::c_int) {
    // SAFETY: _exit is async-signal-safe.
    unsafe { libc::_exit(0) };
}

static SWAP_SINCE_MS: AtomicU64 = AtomicU64::new(0);
const SWAP_LIMIT_MS: u64 = 8000;

fn watch_swaps() {
    let _ = std::thread::Builder::new().name("swap-watch".into()).spawn(|| {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let since = SWAP_SINCE_MS.load(Ordering::Relaxed);
            let now = now_ns() / 1_000_000;
            if since != 0 && now.saturating_sub(since) > SWAP_LIMIT_MS {
                log(&format!("a swap has waited {} ms for a frame callback: ending", now - since));
                unsafe { libc::_exit(0) };
            }
        }
    });
}

fn main() {
    let options = match parse() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(64);
        }
    };
    // SAFETY: plain signal registrations before any thread exists.
    unsafe {
        for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
            libc::signal(signal, on_term as extern "C" fn(libc::c_int) as libc::sighandler_t);
        }
    }
    let shm = options.shm.clone();
    let code = match run(options) {
        Ok(code) => code,
        Err(e) => {
            log(&format!("failed: {e}"));
            1
        }
    };
    let _ = std::fs::remove_dir_all(shm);
    std::process::exit(code);
}

fn graphics(gles: &Gles) -> String {
    let path = match std::env::var("LUMATV_GFX_PATH").as_deref() {
        Ok("64") => "64-bit",
        Ok("32") => "32-bit",
        _ => "host",
    };
    format!("{path} {}", gles.describe())
}

/// The pad's side channel: JSON datagrams from the engine while the stream's input is paused (gfn/engine `pads.rs`), turned into
/// the overlay's keys. A report that moves the left stick or the d-pad is one key; the edge of a button press is one key.
struct PadSide {
    sock: UnixDatagram,
    last_buttons: u16,
    last_dir: Option<PadKey>,
    /// The newest press latency the engine reported, until the loop takes it.
    latency: Option<PadLatency>,
}

impl PadSide {
    fn open(path: &PathBuf) -> Option<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::remove_file(path);
        let sock = UnixDatagram::bind(path).ok()?;
        sock.set_nonblocking(true).ok()?;
        Some(Self { sock, last_buttons: 0, last_dir: None, latency: None })
    }

    fn poll(&mut self) -> Vec<PadKey> {
        let mut out = Vec::new();
        let mut buf = [0u8; 512];
        while let Ok(n) = self.sock.recv(&mut buf) {
            let Ok(v) = serde_json::from_slice::<Value>(&buf[..n]) else { continue };
            match v["pad"].as_str() {
                Some("guide") if v["pressed"] == true => out.push(PadKey::Guide),
                Some("exit") => out.push(PadKey::Exit),
                Some("latency") => {
                    let us = |k: &str| v[k].as_u64().unwrap_or(0).min(u64::from(u32::MAX)) as u32;
                    if v["n"].as_u64().unwrap_or(0) > 0 {
                        self.latency = Some(PadLatency { p50_us: us("p50"), p99_us: us("p99") });
                    }
                }
                Some("report") => {
                    let buttons = v["buttons"].as_u64().unwrap_or(0) as u16;
                    let pressed = buttons & !self.last_buttons;
                    self.last_buttons = buttons;
                    if pressed & 0x1000 != 0 {
                        out.push(PadKey::Ok);
                    }
                    if pressed & 0x2000 != 0 {
                        out.push(PadKey::Back);
                    }
                    let lx = v["lx"].as_i64().unwrap_or(0);
                    let ly = v["ly"].as_i64().unwrap_or(0);
                    let dir = if pressed & 0x0001 != 0 || ly > 20_000 {
                        Some(PadKey::Up)
                    } else if pressed & 0x0002 != 0 || ly < -20_000 {
                        Some(PadKey::Down)
                    } else if pressed & 0x0004 != 0 || lx < -20_000 {
                        Some(PadKey::Left)
                    } else if pressed & 0x0008 != 0 || lx > 20_000 {
                        Some(PadKey::Right)
                    } else {
                        None
                    };
                    if dir != self.last_dir {
                        if let Some(d) = dir {
                            out.push(d);
                        }
                        self.last_dir = dir;
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// Where the core keeps its account files and the app its preferences: the directory asked for if this user can write there, else a
/// directory under the temporary one (the account then has to be signed in again after a restart of the TV). SAM's jail may not
/// give the app a persistent directory of its own.
fn writable_data_dir(wanted: &PathBuf) -> PathBuf {
    let probe = |dir: &PathBuf| -> bool {
        let file = dir.join(format!(".write-test-{}", std::process::id()));
        std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&file, b"x")).is_ok() && std::fs::remove_file(&file).is_ok()
    };
    if probe(wanted) {
        return wanted.clone();
    }
    let fallback = std::env::temp_dir().join("lumatv-gfn-data");
    log(&format!("{} is not writable: the account and the preferences go to {}, which does not survive a restart", wanted.display(), fallback.display()));
    fallback
}

fn run(o: Options) -> io::Result<i32> {
    log(&format!("start: {} core {} engine {} data {}", o.app_id, o.core.display(), o.engine.display(), o.data.display()));
    let _ = std::fs::remove_dir_all(&o.shm);
    std::fs::create_dir_all(&o.shm)?;
    let libmali = o.libmali.as_deref().ok_or_else(|| io::Error::other("--libmali (or LUMATV_GFX_LIB) names the GL driver"))?;
    let spec = Spec { app_id: o.app_id.clone(), title: "GeForce NOW".into(), window_type: "_WEBOS_WINDOW_TYPE_CARD".into(), keys_back: true, state: if o.hidden { None } else { Some(FULLSCREEN) }, keys: true, video: true };
    let (gles, mut win) = window::open(W as u32, H as u32, libmali, &spec)?;
    log(&format!("GL: {}", graphics(&gles)));
    let (seats, keyboards) = win.seats();
    log(&format!("seats: {seats} bound, {keyboards} with a keyboard"));
    // The video element's id goes to the feeder through a file it reads at every load (gfn/feeder, --window-id-file).
    match win.video_window_id() {
        Some(id) => {
            if let Some(dir) = o.window_id_file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            match std::fs::write(&o.window_id_file, format!("{id}\n")) {
                Ok(()) => log(&format!("video element {id} written to {}", o.window_id_file.display())),
                Err(e) => log(&format!("cannot write {}: {e}", o.window_id_file.display())),
            }
        }
        None => log("no video element id from the compositor: the stream will have nowhere to show"),
    }
    let (mut engine, app_end) = Engine::new(
        gles,
        HostConfig { width: W as u32, height: H as u32, refresh_hz: 60.0, shm_dir: o.shm.clone(), direct: false, gpu_sync: false, idle_trim: true, pages: 0, reference: false, blit: true, external: false, system_z: i32::MAX },
    )?;
    // Over the stream the window is clear but for the status line and the overlay: the video plane shows through everywhere else.
    engine.set_transparent(true);
    let (scene, _welcome, events) = ui::Scene::over_with(app_end, SceneOptions { shm_dir: Some(o.shm.clone()), ..SceneOptions::default() })?;
    engine.finish_handshake()?;
    let mut ui = Ui::new(scene);
    ui.enable_glyph_text().map_err(|e| io::Error::other(format!("{e}")))?;
    let mut land = |ui: &mut Ui| -> io::Result<()> {
        let began = Instant::now();
        loop {
            while let Ok(event) = events.try_recv() {
                ui.scene.observe(&event);
            }
            if let Some(atlas) = ui.text_atlas() {
                atlas.pump();
            }
            if ui.scene.has_pending() {
                let _ = ui.try_commit()?;
            }
            engine.pump()?;
            engine.apply_waiting(now_ns());
            engine.tidy();
            if ui.glyph_text_settled() && !ui.scene.has_pending() && !engine.tidy_more() {
                return Ok(());
            }
            if began.elapsed() > Duration::from_secs(5) {
                return Err(io::Error::other("the glyph atlas did not land in 5 s"));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let mut page = GfnUi::build(ui, &o.assets, o.text_scale, o.mode, None, &mut land)?;
    land(&mut page.ui)?;

    // The core, with the engine's whereabouts and the feeder's socket in its environment (the engine reads them; the core passes
    // its environment on). The engine reads the dead zone from the preferences file when a stream starts.
    let data = writable_data_dir(&o.data);
    let prefs_path = o.prefs.clone().unwrap_or_else(|| data.join("prefs.json"));
    let env = [
        ("LUMATV_GFN_FEEDER_SOCKET", o.feeder_socket.to_string_lossy().into_owned()),
        ("LUMATV_GFN_PAD_SIDE", o.pad_side.to_string_lossy().into_owned()),
        ("LUMATV_GFN_PREFS", prefs_path.to_string_lossy().into_owned()),
        ("OPENNOW_STREAMER_LOG_FILE", data.join("streamer.log").to_string_lossy().into_owned()),
        // Where the engine writes latency.tsv and latency.json (one line per button press); `lumatv-gfn-latency` reads them.
        ("LUMATV_GFN_LATENCY_DIR", data.to_string_lossy().into_owned()),
    ];
    let mut core = CoreClient::spawn(&o.core, &o.core_args, &data, &env)?;
    log(&format!("core started, pid {:?}, data {}", core.child_pid(), data.display()));
    let mut pad = PadSide::open(&o.pad_side);
    // SAFETY: the feeder made the descriptor and gave it to this process alone.
    let mut channel = match o.channel_fd {
        Some(fd) => Some(unsafe { Channel::from_fd(fd)? }),
        None => None,
    };
    let mut window_state = if o.hidden { MINIMIZED } else { FULLSCREEN };
    let mut app = App::with_prefs(Prefs::load(&prefs_path)).with_engine(&o.engine.to_string_lossy());
    let started = Instant::now();
    let mut actions = app.start();
    if o.hidden {
        actions.extend(app.handle(Input::State(MINIMIZED)));
    }
    watch_swaps();
    let mut want_frame = true;
    let mut ready = false;
    loop {
        for action in actions.drain(..) {
            match action {
                Action::Request { method, params } => {
                    let shown = params.clone();
                    match core.request(&method, params) {
                        Ok(id) => app.sent_params(&id, &method, &shown),
                        Err(e) => log(&format!("core request {method}: {e}")),
                    }
                }
                Action::SavePrefs(p) => {
                    if let Err(e) = p.save(&prefs_path) {
                        log(&format!("cannot write {}: {e}", prefs_path.display()));
                    }
                }
                Action::Ack { id } => {
                    let _ = core.ack(&id);
                }
                Action::PauseInput(_) => {}
                Action::Exit(why) => {
                    log(&format!("exit: {why}"));
                    if let Some(ch) = channel.as_mut() {
                        ch.send(&FromApp::Exit(why));
                    }
                    core.shutdown();
                    return Ok(0);
                }
            }
        }
        let mut inputs: Vec<Input> = Vec::new();
        for e in win.events() {
            match e {
                Event::State(s) => {
                    window_state = s;
                    if let Some(ch) = channel.as_mut() {
                        ch.send(&FromApp::State(s));
                    }
                    inputs.push(Input::State(s));
                }
                Event::Close => inputs.push(Input::Close),
                Event::Key { code, pressed } => {
                    if key_of(code).is_some() {
                        inputs.push(Input::Key { code, pressed });
                    } else if pressed {
                        log(&format!("key {code}"));
                    }
                }
            }
        }
        if let Some(ch) = channel.as_mut() {
            match ch.read() {
                Ok(messages) => {
                    for m in messages {
                        match m {
                            ToApp::Close { reason } => {
                                log(&format!("SAM asks the app to close ({reason})"));
                                inputs.push(Input::Close);
                            }
                            ToApp::Relaunch { reason, parameters } => {
                                log(&format!("SAM launched the app again ({reason}) {parameters}"));
                                // A relaunch is the user asking for the app: a covered window is brought forward (the compositor's
                                // word follows and corrects this).
                                if window_state == MINIMIZED {
                                    win.request_state(FULLSCREEN);
                                    window_state = FULLSCREEN;
                                    inputs.push(Input::State(FULLSCREEN));
                                }
                            }
                            ToApp::Sam { registered } => log(&format!("SAM registration: {}", if registered { "accepted" } else { "not accepted: the app runs without SAM's events" })),
                        }
                    }
                }
                // the feeder is the app SAM knows: without it nothing is left to answer to
                Err(_) => {
                    log("the feeder closed the channel");
                    inputs.push(Input::Close);
                    channel = None;
                }
            }
        }
        if let Some(p) = pad.as_mut() {
            inputs.extend(p.poll().into_iter().map(Input::Pad));
            if let Some(l) = p.latency.take() {
                inputs.push(Input::PadLatency(l));
            }
        }
        inputs.extend(core.poll().into_iter().map(Input::Core));
        inputs.push(Input::Tick { now_ms: started.elapsed().as_millis() as u64 });
        for input in inputs {
            if !matches!(input, Input::Tick { .. }) {
                log(&format!("input {}", describe(&input)));
            }
            actions.extend(app.handle(input));
        }
        if !actions.is_empty() {
            continue;
        }

        let visible = app.wants_frames();
        let now = now_ns();
        if visible {
            while let Ok(event) = events.try_recv() {
                page.ui.scene.observe(&event);
            }
            if let Some(atlas) = page.ui.text_atlas() {
                atlas.pump();
            }
            page.show(&app);
            if page.ui.scene.has_pending() {
                let _ = page.ui.try_commit()?;
            }
            if engine.pump()?.closed {
                return Err(io::Error::other("the compositor side of the toolkit closed"));
            }
            let demand = engine.demand();
            let due = match demand {
                Demand::EveryRefresh => true,
                Demand::At(slot) => now >= slot,
                Demand::Idle => false,
            };
            if want_frame || engine.has_commits() || engine.redraw_wanted() || due {
                want_frame = false;
                let cost = engine.draw(now_ns());
                if let Some(code) = cost.gl_error {
                    log(&format!("GL error {code:#x}"));
                }
                SWAP_SINCE_MS.store((now_ns() / 1_000_000).max(1), Ordering::Relaxed);
                let swapped = engine.gles().swap_buffers();
                SWAP_SINCE_MS.store(0, Ordering::Relaxed);
                swapped?;
                engine.frame_done(now_ns(), &cost);
                engine.tidy();
                engine.idle_moment(true);
                if !ready {
                    ready = true;
                    log("first frame presented");
                    if let Some(ch) = channel.as_mut() {
                        ch.send(&FromApp::Ready);
                    }
                }
                continue;
            }
            engine.apply_waiting(now);
            if engine.tidy_more() {
                engine.tidy();
                continue;
            }
            engine.idle_moment(false);
        } else {
            want_frame = true;
        }

        let mut wait_ms = 250i64;    /* the core's messages arrive on a channel with no descriptor to wait on */
        if page.art_busy() {
            wait_ms = wait_ms.min(16);   /* a decoded picture arrives on a channel too: look for it every frame while any is in the works */
        }
        if let Some(d) = app.next_wake() {
            wait_ms = wait_ms.min(d.as_millis() as i64);
        }
        if visible {
            let demand = engine.demand();
            if let Demand::At(slot) = demand {
                wait_ms = wait_ms.min((slot.saturating_sub(now_ns()) / 1_000_000) as i64);
            }
            if let Some(wake) = engine.idle_wake_at(demand == Demand::Idle) {
                wait_ms = wait_ms.min((wake.saturating_sub(now_ns()) / 1_000_000) as i64);
            }
        }
        let mut extra = [-1; 2];
        if let Some(p) = pad.as_ref() {
            extra[0] = p.sock.as_raw_fd();
        }
        if let Some(ch) = channel.as_ref() {
            extra[1] = ch.fd();
        }
        win.wait_fds(wait_ms.max(0) as i32, &extra)?;
    }
}

fn describe(input: &Input) -> String {
    match input {
        Input::Key { code, pressed } => format!("key {code} {}", if *pressed { "down" } else { "up" }),
        Input::Pad(p) => format!("pad {p:?}"),
        Input::PadLatency(l) => format!("pad latency {l:?}"),
        Input::Core(CoreMsg::Response { method, ok, error_code, .. }) => format!("{method}: {}", if *ok { "ok".to_owned() } else { format!("error {error_code}") }),
        Input::Core(CoreMsg::Event { name, .. }) => format!("event {name}"),
        Input::Core(CoreMsg::Gone) => "core gone".into(),
        Input::State(s) => format!("state {s}"),
        Input::Close => "close".into(),
        Input::Tick { .. } => "tick".into(),
    }
}
