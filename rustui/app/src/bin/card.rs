//! `lumatv-card-app`: the first LumaTV app, a 64-bit card window of LG's compositor (and, unchanged, of LumaTV's) drawn with LumaShow's
//! toolkit. SAM starts the 32-bit launcher (`stub/lumatv-card.c`), which starts this through `gfx64/launch.sh` and hands SAM's
//! `relaunch` and `close` on over `--channel-fd`. The rules are `card.rs`'s, the page `card_ui.rs`'s; this is the loop between them and
//! the compositor. `docs/research/cardapp-result.md` has the account.
//!
//!   lumatv-card-app [--app-id ID] [--title T] [--reason R] [--hidden] [--params JSON] [--channel-fd N] [--assets DIR] [--shm DIR]
//!                   [--libmali PATH] [--window-type T]
//!
//! `--hidden` is a preload: the window is made but asked for nothing and drawn on never, until SAM relaunches the app.

use lumatv_render::host::{Engine, HostConfig};
use lumatv_render::{Demand, Gles, now_ns};
use lumatv_rustui::card::{Card, FULLSCREEN, Input};
use lumatv_rustui::card_ui::{CardUi, H, W};
use lumatv_rustui::lifecycle::{Channel, FromApp, ToApp};
use lumatv_rustui::window::{self, Event, Spec};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use ui::{SceneOptions, Ui};

struct Options {
    app_id: String,
    title: String,
    reason: String,
    hidden: bool,
    params: String,
    channel_fd: Option<i32>,
    assets: PathBuf,
    shm: PathBuf,
    libmali: Option<String>,
    window_type: String,
}

fn parse() -> Result<Options, String> {
    let mut o = Options {
        app_id: "org.nphil.lumatv.card".into(),
        title: "LumaTV Card".into(),
        reason: "undefined".into(),
        hidden: false,
        params: "{}".into(),
        channel_fd: None,
        assets: PathBuf::from("assets"),
        shm: std::env::temp_dir().join("lumatv-card-shm"),
        libmali: std::env::var("LUMATV_GFX_LIB").ok(),
        window_type: "_WEBOS_WINDOW_TYPE_CARD".into(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--app-id" => o.app_id = value("--app-id")?,
            "--title" => o.title = value("--title")?,
            "--reason" => o.reason = value("--reason")?,
            "--hidden" => o.hidden = true,
            "--params" => o.params = value("--params")?,
            "--channel-fd" => o.channel_fd = Some(value("--channel-fd")?.parse().map_err(|e| format!("--channel-fd: {e}"))?),
            "--assets" => o.assets = PathBuf::from(value("--assets")?),
            "--shm" => o.shm = PathBuf::from(value("--shm")?),
            "--libmali" => o.libmali = Some(value("--libmali")?),
            "--window-type" => o.window_type = value("--window-type")?,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

fn log(what: &str) {
    eprintln!("card[{:.3}]: {what}", now_ns() as f64 / 1e9);
}

/// SAM or the launcher stops the app with SIGTERM. A swap that waits for a frame callback the compositor never sends (a hidden
/// card on LG's compositor [INFERENCE]) must not keep the app past that, so the handler ends the process itself.
extern "C" fn on_term(_: libc::c_int) {
    // SAFETY: _exit is async-signal-safe.
    unsafe { libc::_exit(0) };
}

/// When the loop went into a swap, in ms of the monotonic clock; 0 while it is not in one.
static SWAP_SINCE_MS: AtomicU64 = AtomicU64::new(0);
/// A swap waits for the compositor's frame callback. A card whose compositor stops sending them without telling it (LG's, for a card it
/// hides, between the app's last frame and its `state_changed(minimized)` [INFERENCE]) would wait for ever and hear neither SAM nor a
/// relaunch. Past this the process ends, which SAM and the compositor take as the card closing, and a launch starts it afresh. Power-off
/// closes native apps through SAM (`close`, reason `powerOff`) well before it matters.
const SWAP_LIMIT_MS: u64 = 8000;

fn watch_swaps() {
    let spawned = std::thread::Builder::new().name("swap-watch".into()).spawn(|| {
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let since = SWAP_SINCE_MS.load(Ordering::Relaxed);
            let now = now_ns() / 1_000_000;
            if since != 0 && now.saturating_sub(since) > SWAP_LIMIT_MS {
                log(&format!("a swap has waited {} ms for a frame callback: ending", now - since));
                // SAFETY: _exit is async-signal-safe and takes nothing with it that a thread's exit would leave in a state.
                unsafe { libc::_exit(0) };
            }
        }
    });
    if let Err(e) = spawned {
        log(&format!("no swap watchdog: {e}"));
    }
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
    // Nothing is torn down: the GL context and the Wayland connection go with the process, as in the spike. The picture files do not.
    let _ = std::fs::remove_dir_all(shm);
    std::process::exit(code);
}

/// What the page says about the graphics path: the driver's renderer and which path got the app there.
fn graphics(gles: &Gles) -> String {
    let describe = gles.describe();
    let renderer = describe.split(" | ").next().unwrap_or("").trim();
    match std::env::var("LUMATV_GFX_PATH").as_deref() {
        Ok("64") => format!("{renderer}, 64-bit driver"),
        Ok("32") => format!("{renderer}, LG's 32-bit driver"),
        _ => format!("{renderer}, host"),
    }
}

fn run(o: Options) -> io::Result<i32> {
    log(&format!("start: {} reason {} hidden {} params {} gfx {:?}", o.app_id, o.reason, o.hidden, o.params, std::env::var("LUMATV_GFX_PATH").ok()));
    let mut channel = match o.channel_fd {
        // SAFETY: the launcher made this descriptor for the app and nothing else in the process closes it.
        Some(fd) => Some(unsafe { Channel::from_fd(fd)? }),
        None => None,
    };
    let mut card = Card::new(o.hidden, &o.reason, channel.is_some());
    let _ = std::fs::remove_dir_all(&o.shm);
    std::fs::create_dir_all(&o.shm)?;

    let libmali = o.libmali.as_deref().ok_or_else(|| io::Error::other("--libmali (or LUMATV_GFX_LIB) names the GL driver"))?;
    // KEYS_BACK: the card takes Back itself (goes up a page, then ends); Exit (Back held) stays the system's and closes the app.
    let spec = Spec {
        app_id: o.app_id.clone(),
        title: o.title.clone(),
        window_type: o.window_type.clone(),
        keys_back: true,
        state: if o.hidden { None } else { Some(FULLSCREEN) },
        keys: true,
        video: false,
    };
    let (gles, mut win) = window::open(W as u32, H as u32, libmali, &spec)?;
    let graphics = graphics(&gles);
    log(&format!("GL: {}", gles.describe()));
    let (seats, keyboards) = win.seats();
    log(&format!("seats: {seats} bound, {keyboards} with a keyboard"));
    let (mut engine, app_end) = Engine::new(
        gles,
        HostConfig {
            width: W as u32,
            height: H as u32,
            refresh_hz: 60.0,
            shm_dir: o.shm.clone(),
            direct: false,
            gpu_sync: false,
            idle_trim: true,
            pages: 0,
            reference: false,
            blit: true,
            external: false,
            system_z: i32::MAX,
        },
    )?;
    let (scene, _welcome, events) = ui::Scene::over_with(app_end, SceneOptions { shm_dir: Some(o.shm.clone()), ..SceneOptions::default() })?;
    engine.finish_handshake()?;
    let mut ui = Ui::new(scene);
    ui.enable_glyph_text().map_err(|e| io::Error::other(format!("{e}")))?;
    let built = Instant::now();
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
    let mut page = CardUi::build(ui, &o.assets, &mut land)?;
    land(&mut page.ui)?;
    log(&format!("built the page in {} ms", built.elapsed().as_millis()));

    watch_swaps();
    let began = Instant::now();
    let mut visible = false;
    let mut want_frame = false;
    let mut ready = false;
    loop {
        // What the compositor and the launcher said.
        let mut inputs: Vec<Input> = win
            .events()
            .into_iter()
            .map(|e| match e {
                Event::State(s) => Input::State(s),
                Event::Close => Input::CompositorClose,
                Event::Key { code, pressed } => Input::Key { code, pressed },
            })
            .collect();
        if let Some(ch) = channel.as_mut() {
            match ch.read() {
                Ok(messages) => inputs.extend(messages.into_iter().map(|m| match m {
                    ToApp::Relaunch { reason, parameters } => Input::Relaunch { reason, parameters },
                    ToApp::Close { reason } => Input::SamClose { reason },
                    ToApp::Sam { registered } => Input::Sam { registered },
                })),
                Err(_) => inputs.push(Input::LauncherGone),
            }
        }
        for input in inputs {
            log(&format!("input {input:?}"));
            let out = card.handle(input);
            if let Some(ch) = channel.as_mut() {
                for msg in &out.report {
                    ch.send(msg);
                }
            }
            if let Some(state) = out.request_state {
                log(&format!("asking the compositor for state {state}"));
                win.request_state(state);
            }
            if let Some(exit) = out.exit {
                let why = exit.why();
                log(&format!("exit: {why}"));
                if let Some(ch) = channel.as_mut() {
                    ch.send(&FromApp::Exit(why));
                }
                return Ok(0);
            }
        }

        // A card that becomes visible again is drawn at once: what it missed is on the page by then.
        if card.wants_frames() && !visible {
            want_frame = true;
        }
        visible = card.wants_frames();
        let now = now_ns();
        if visible {
            while let Ok(event) = events.try_recv() {
                page.ui.scene.observe(&event);
            }
            if let Some(atlas) = page.ui.text_atlas() {
                atlas.pump();
            }
            page.show(&card, began.elapsed().as_secs(), &graphics);
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
                // Only a visible card swaps: a swap waits for a frame callback, which a covered card may never get.
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
        }

        // Nothing is due: sleep until the next thing is, or until the compositor or the launcher says something.
        let mut wait_ms = 1000i64;
        if visible {
            // the uptime on the page changes on the second
            let to_second = 1000 - i64::from(began.elapsed().subsec_millis());
            wait_ms = wait_ms.min(to_second + 1);
            let demand = engine.demand();
            if let Demand::At(slot) = demand {
                wait_ms = wait_ms.min((slot.saturating_sub(now_ns()) / 1_000_000) as i64);
            }
            if let Some(wake) = engine.idle_wake_at(demand == Demand::Idle) {
                wait_ms = wait_ms.min((wake.saturating_sub(now_ns()) / 1_000_000) as i64);
            }
        }
        let extra = channel.as_ref().map(Channel::fd);
        win.wait(wait_ms.max(0) as i32, extra)?;
    }
}
