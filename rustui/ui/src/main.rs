//! `lumatv-ui`: the shell's logic process. On the set the compositor starts it with the engine's socket (`LUMATV_UI_FD`), the key
//! socket (`LUMATV_UI_KEYS_FD`), the UI channel (`LUMATV_SHELL_PORT`, `LUMATV_SHELL_TOKEN_FILE`) and the kinds it answers
//! (`LUMATV_UI_KINDS`); on a workstation `--host` runs the same shell against the engine in this process on Mesa (the renderer as
//! the compositor runs it) and renders what it draws to PNG, against the compositor's stand-in channel (`--port`, as
//! `tests/qmlhost/channel.py` serves it) or a script of pushes of its own.
//!
//!   lumatv-ui [--kinds osd,toast] [--data-dir DIR] [--module-dir DIR] [--shm DIR] [--system-z N] [--text-scale 1..2] [--mode dark|light]
//!   lumatv-ui --host [--port N --token-file F] [--helper-port N] [--wait MS] [--push MS:topic:JSON] [--luna MS:TAG:JSON]
//!             [--helper MS:topic:JSON] [--key MS:name[:up|:repeat]] [--pointer MS:x:y[:buttons[:hidden]]]
//!             [--eval MS:state|geometry|report] [--snap MS:FILE.png] [--frames MS:LABEL] [--backdrop FILE.png]
//!             [--theme JSON] [--log FILE] [--reference]
//!
//! Host output lines: `[report] <json>` (what went to `POST /v1/ui`), `[luna] <json>`, `[helper] <json>`, `[eval] <json>`,
//! `[frames] LABEL N`, `shot FILE`, `[frame] ...` per frame with `--log`, and `RESULT frames N gpu mean/max us ...` at the end.

use compositor_protocol::Event as SceneEvent;
use lumatv_render::host::{Engine, HostConfig};
use lumatv_render::{Demand, Gles};
use lumatv_tokens::Tokens;
use lumatv_ui::host::http::{Event, Feed};
use lumatv_ui::host::keys::{self, KeyEvent};
use lumatv_ui::host::now_ms;
use lumatv_ui::kit::Kit;
use lumatv_ui::shell::{Act, Kinds, Shell};
use serde_json::{Value, json};
use std::io::{self, Write};
use std::os::fd::RawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use ui::lucent::material::MaterialTier;
use ui::lucent::palette::Mode;
use ui::scene::{Scene, SceneOptions};

const SHIPPED_LAYOUT: &str = include_str!("../../../payload/theme/layout.json");
/// The helper's topics the shell follows besides its plugins' (`kit/Helper.qml` `subscribed`, and the sheets of the cameras).
const HELPER_TOPICS: &str =
    "screen,weather,weather.more,news,receiver,ai,ai.prompt,plugins,ha.link,updateblock,openapps,panel,cameras,cameras.popup,cameras.live,cameras.events,profiles,voice,saver";

struct Options {
    host: bool,
    kinds: String,
    data_dir: PathBuf,
    module_dir: PathBuf,
    shm: PathBuf,
    system_z: i32,
    text_scale: f32,
    mode: Option<Mode>,
    port: Option<u16>,
    token_file: Option<PathBuf>,
    helper_port: u16,
    wait_ms: u64,
    pushes: Vec<(u64, String, Value)>,
    lunas: Vec<(u64, u64, Value)>,
    helpers: Vec<(u64, String, Value)>,
    keys: Vec<(u64, KeyEvent)>,
    evals: Vec<(u64, String)>,
    snaps: Vec<(u64, PathBuf)>,
    frames: Vec<(u64, String)>,
    backdrop: Option<PathBuf>,
    theme: Option<Value>,
    log: Option<PathBuf>,
    clock: Option<u64>,
    /// Every frame drawn whole as well and compared with the one drawn into the ring (`--reference`).
    reference: bool,
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn parse() -> Result<Options, String> {
    let mut o = Options {
        host: false,
        kinds: std::env::var("LUMATV_UI_KINDS").unwrap_or_else(|_| "osd,toast".into()),
        data_dir: env_path("LUMATV_DATA_DIR").unwrap_or_else(|| PathBuf::from("/var/lib/lumatv")),
        module_dir: env_path("LUMATV_MODULE_DIR").unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../payload/compositor/lumatv"))),
        shm: env_path("LUMATV_UI_SHM").unwrap_or_else(|| std::env::temp_dir().join("lumatv-ui-shm")),
        system_z: std::env::var("LUMATV_UI_SYSTEM_Z").ok().and_then(|s| s.parse().ok()).unwrap_or(1000),
        text_scale: 1.0,
        mode: None,
        port: std::env::var("LUMATV_SHELL_PORT").ok().and_then(|s| s.parse().ok()),
        token_file: env_path("LUMATV_SHELL_TOKEN_FILE"),
        helper_port: 18731,
        wait_ms: 6000,
        pushes: Vec::new(),
        lunas: Vec::new(),
        helpers: Vec::new(),
        keys: Vec::new(),
        evals: Vec::new(),
        snaps: Vec::new(),
        frames: Vec::new(),
        backdrop: None,
        theme: None,
        log: None,
        clock: None,
        reference: false,
    };
    let mut args = std::env::args().skip(1);
    let value = |args: &mut std::iter::Skip<std::env::Args>, flag: &str| args.next().ok_or_else(|| format!("{flag} needs a value"));
    while let Some(a) = args.next() {
        match a.as_str() {
            "--host" => o.host = true,
            "--reference" => o.reference = true,
            "--kinds" => o.kinds = value(&mut args, "--kinds")?,
            "--data-dir" => o.data_dir = PathBuf::from(value(&mut args, "--data-dir")?),
            "--module-dir" => o.module_dir = PathBuf::from(value(&mut args, "--module-dir")?),
            "--shm" => o.shm = PathBuf::from(value(&mut args, "--shm")?),
            "--system-z" => o.system_z = value(&mut args, "--system-z")?.parse().map_err(|_| "--system-z")?,
            "--text-scale" => o.text_scale = value(&mut args, "--text-scale")?.parse().map_err(|_| "--text-scale")?,
            "--mode" => {
                o.mode = Some(match value(&mut args, "--mode")?.as_str() {
                    "light" => Mode::Light,
                    _ => Mode::Dark,
                })
            }
            "--port" => o.port = Some(value(&mut args, "--port")?.parse().map_err(|_| "--port")?),
            "--token-file" => o.token_file = Some(PathBuf::from(value(&mut args, "--token-file")?)),
            "--helper-port" => o.helper_port = value(&mut args, "--helper-port")?.parse().map_err(|_| "--helper-port")?,
            "--wait" => o.wait_ms = value(&mut args, "--wait")?.parse().map_err(|_| "--wait")?,
            "--push" | "--helper" => {
                let spec = value(&mut args, &a)?;
                let mut parts = spec.splitn(3, ':');
                let ms: u64 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| format!("{a} MS:topic:JSON"))?;
                let topic = parts.next().ok_or_else(|| format!("{a} MS:topic:JSON"))?.to_string();
                let doc: Value = serde_json::from_str(parts.next().unwrap_or("null")).map_err(|e| format!("{a}: {e}"))?;
                if a == "--push" { o.pushes.push((ms, topic, doc)) } else { o.helpers.push((ms, topic, doc)) }
            }
            "--luna" => {
                let spec = value(&mut args, "--luna")?;
                let mut parts = spec.splitn(3, ':');
                let ms: u64 = parts.next().and_then(|s| s.parse().ok()).ok_or("--luna MS:TAG:JSON")?;
                let tag: u64 = parts.next().and_then(|s| s.parse().ok()).ok_or("--luna MS:TAG:JSON")?;
                let doc: Value = serde_json::from_str(parts.next().unwrap_or("null")).map_err(|e| format!("--luna: {e}"))?;
                o.lunas.push((ms, tag, doc));
            }
            "--key" => {
                // MS:name, MS:name:up (a release), MS:name:repeat
                let spec = value(&mut args, "--key")?;
                let mut parts = spec.splitn(3, ':');
                let ms: u64 = parts.next().and_then(|s| s.parse().ok()).ok_or("--key MS:name[:up|:repeat]")?;
                let name = parts.next().filter(|n| !n.is_empty()).ok_or("--key MS:name[:up|:repeat]")?.to_string();
                let how = parts.next().unwrap_or("");
                o.keys.push((ms, KeyEvent::Key { name, down: how != "up", repeat: how == "repeat", seat: 0 }));
            }
            "--pointer" => {
                // MS:x:y, MS:x:y:buttons, MS:x:y:buttons:hidden
                let spec = value(&mut args, "--pointer")?;
                let mut parts = spec.splitn(5, ':');
                let bad = || "--pointer MS:x:y[:buttons[:hidden]]";
                let ms: u64 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(bad)?;
                let x: f32 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(bad)?;
                let y: f32 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(bad)?;
                let buttons: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let visible = parts.next() != Some("hidden");
                o.keys.push((ms, KeyEvent::Pointer { x, y, buttons, visible }));
            }
            "--eval" | "--snap" | "--frames" => {
                let spec = value(&mut args, &a)?;
                let (ms, rest) = spec.split_once(':').ok_or_else(|| format!("{a} MS:VALUE"))?;
                let ms: u64 = ms.parse().map_err(|_| format!("{a} MS:VALUE"))?;
                match a.as_str() {
                    "--eval" => o.evals.push((ms, rest.to_string())),
                    "--snap" => o.snaps.push((ms, PathBuf::from(rest))),
                    _ => o.frames.push((ms, rest.to_string())),
                }
            }
            "--backdrop" => o.backdrop = Some(PathBuf::from(value(&mut args, "--backdrop")?)),
            "--theme" => o.theme = Some(serde_json::from_str(&value(&mut args, "--theme")?).map_err(|e| format!("--theme: {e}"))?),
            "--log" => o.log = Some(PathBuf::from(value(&mut args, "--log")?)),
            "--clock" => {
                let text = value(&mut args, "--clock")?;
                o.clock = Some(lumatv_ui::host::clock::parse_local(&text).ok_or("--clock YYYY-MM-DDTHH:MM[:SS]")?);
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

/// One step of the harness's script: what it does to the shell, the engine and the log at its time.
type Step = Box<dyn FnOnce(&mut Shell, &mut Engine, &mut Log) -> Vec<Act>>;

/// Everything the loop waits for, on one channel.
enum Msg {
    Channel(Event),
    Helper(Event),
    Key(KeyEvent),
    Scene(SceneEvent),
}

fn forward<T: Send + 'static>(rx: Receiver<T>, tx: Sender<Msg>, wrap: fn(T) -> Msg) {
    std::thread::spawn(move || {
        for ev in rx {
            if tx.send(wrap(ev)).is_err() {
                break;
            }
        }
    });
}

struct Log {
    file: Option<std::fs::File>,
}

impl Log {
    fn line(&mut self, text: &str) {
        println!("{text}");
        let _ = io::stdout().flush();
        if let Some(f) = &mut self.file {
            let _ = writeln!(f, "{text}");
        }
    }
}

/// The theme and Home's layout from the data directory, with `--theme` merged over.
fn theme_and_layout(o: &Options) -> (Tokens, Value, Value) {
    let mut user = std::fs::read(o.data_dir.join("theme/theme.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(Value::Null);
    if let Some(t) = &o.theme {
        if user.is_null() {
            user = t.clone();
        } else {
            lumatv_tokens::merge(&mut user, t);
        }
    }
    let tokens = Tokens::from_json(&user);
    let theme = tokens.json().clone();
    let layout = std::fs::read(o.data_dir.join("theme/layout.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::from_str(SHIPPED_LAYOUT).expect("the shipped layout is JSON"));
    (tokens, theme, layout)
}

fn make_shell(o: &Options, scene: Scene) -> io::Result<Shell> {
    let (tokens, theme, layout) = theme_and_layout(o);
    let mode = o.mode.unwrap_or_else(|| tokens.mode(None));
    let kit = Kit::new(scene, tokens, mode, MaterialTier::T1, o.text_scale)?;
    let mut shell = Shell::new(kit, Kinds::parse(&o.kinds), o.system_z, &o.module_dir.join("icons/audio"), &theme, &layout)?;
    shell.dirs(&o.module_dir, &o.data_dir);
    Ok(shell)
}

/// Carries out what the shell asked: the channel's and the helper's requests, the log.
fn carry(acts: Vec<Act>, channel: Option<&Feed>, helper: Option<&Feed>, log: &mut Log) {
    for act in acts {
        match act {
            Act::Channel(out) => {
                if out.path == "/v1/ui" {
                    log.line(&format!("[report] {}", out.body.as_ref().map_or(String::new(), Value::to_string)));
                } else if out.path == "/v1/luna" {
                    log.line(&format!("[luna] {}", out.body.as_ref().map_or(String::new(), Value::to_string)));
                } else if out.path == "/v1/switcher" {
                    log.line(&format!("[switcher] {}", out.body.as_ref().map_or(String::new(), Value::to_string)));
                }
                if let Some(c) = channel {
                    c.request(out);
                }
            }
            Act::Helper(out) => {
                log.line(&format!("[helper] {} {} {}", out.method, out.path, out.body.as_ref().map_or(String::new(), Value::to_string)));
                if let Some(h) = helper {
                    h.request(out);
                }
            }
            Act::Log(line) => log.line(&format!("[log] {line}")),
        }
    }
}

fn main() {
    let o = match parse() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("lumatv-ui: {e}");
            std::process::exit(2);
        }
    };
    let result = if o.host { run_host(o) } else { run_set(o) };
    if let Err(e) = result {
        eprintln!("lumatv-ui: {e}");
        std::process::exit(1);
    }
}

/// On the set: the engine's socket from the compositor, the channel and the helper over the loopback, the keys over the key socket.
fn run_set(o: Options) -> io::Result<()> {
    let mut log = Log { file: None };
    let fd: RawFd =
        std::env::var("LUMATV_UI_FD").ok().and_then(|s| s.parse().ok()).ok_or_else(|| io::Error::other("LUMATV_UI_FD names the engine's socket"))?;
    // SAFETY: the compositor handed this process the descriptor at its start and nothing else holds it.
    let stream = unsafe { <UnixStream as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    std::fs::create_dir_all(&o.shm)?;
    let (scene, welcome, scene_events) = Scene::over_with(stream, SceneOptions { shm_dir: Some(o.shm.clone()), ..SceneOptions::default() })?;
    log.line(&format!("engine: {}x{} at {} Hz", welcome.panel_width, welcome.panel_height, welcome.refresh_hz));
    let token = o.token_file.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.trim().to_string());
    let port = o.port.ok_or_else(|| io::Error::other("LUMATV_SHELL_PORT names the UI channel"))?;
    let (channel, channel_events) = Feed::start("channel", port, token, None)?;
    let (helper, helper_events) = Feed::start_following("helper", o.helper_port, None, HELPER_TOPICS)?;
    let (tx, rx) = mpsc::channel::<Msg>();
    forward(scene_events, tx.clone(), Msg::Scene);
    forward(channel_events, tx.clone(), Msg::Channel);
    forward(helper_events, tx.clone(), Msg::Helper);
    if let Some(kfd) = std::env::var("LUMATV_UI_KEYS_FD").ok().and_then(|s| s.parse::<RawFd>().ok()) {
        // SAFETY: as for the engine's socket.
        let keys = unsafe { keys::listen(kfd) }?;
        forward(keys, tx.clone(), Msg::Key);
    }
    for line in lumatv_ui::host::buttons::migrate(&o.data_dir) {
        log.line(&format!("[buttons] {line}"));
    }
    let mut shell = make_shell(&o, scene)?;
    carry(shell.start(), Some(&channel), Some(&helper), &mut log);
    loop {
        let now = now_ms();
        let timeout = shell.due_at().map_or(Duration::from_secs(5), |due| Duration::from_millis(due.saturating_sub(now).clamp(1, 5000)));
        match rx.recv_timeout(timeout) {
            Ok(Msg::Scene(ev)) => shell.kit.ui.scene.observe(&ev),
            Ok(Msg::Channel(ev)) => carry(shell.channel(&ev, now_ms()), Some(&channel), Some(&helper), &mut log),
            Ok(Msg::Helper(ev)) => carry(shell.helper(&ev, now_ms()), Some(&channel), Some(&helper), &mut log),
            Ok(Msg::Key(KeyEvent::Closed)) => return Err(io::Error::other("the compositor closed the key socket")),
            Ok(Msg::Key(ev)) => carry(shell.key(&ev, now_ms()), Some(&channel), Some(&helper), &mut log),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err(io::Error::other("every feed ended")),
        }
        if shell.due_at().is_some_and(|due| now_ms() >= due) {
            carry(shell.tick(now_ms()), Some(&channel), Some(&helper), &mut log);
        }
        if let Some(atlas) = shell.kit.ui.text_atlas() {
            atlas.pump();
        }
        if shell.kit.ui.scene.has_pending() {
            match shell.kit.ui.try_commit() {
                Ok(_) => {}
                Err(e) => return Err(io::Error::other(format!("the engine's socket: {e}"))),
            }
        }
        let now_ns = shell.kit.ui.scene.now_ns();
        shell.tidy(now_ns);
    }
}

/// `rows` are RGBA from the bottom row up, premultiplied: written top row first, over black, which is what the screen shows.
fn write_png(path: &std::path::Path, width: u32, height: u32, rows: &[u8]) -> io::Result<()> {
    let mut rgba = Vec::with_capacity(rows.len());
    for y in (0..height as usize).rev() {
        let row = &rows[y * width as usize * 4..(y + 1) * width as usize * 4];
        for px in row.chunks_exact(4) {
            rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
    }
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(&rgba)).map_err(io::Error::other)
}

fn read_png(path: &std::path::Path) -> io::Result<(u32, u32, Vec<u8>)> {
    let decoder = png::Decoder::new(io::BufReader::new(std::fs::File::open(path)?));
    let mut reader = decoder.read_info().map_err(io::Error::other)?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(io::Error::other)?;
    let bytes = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => bytes.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::Grayscale => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => bytes.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        other => return Err(io::Error::other(format!("png colour type {other:?}"))),
    };
    Ok((info.width, info.height, rgba))
}

/// A host run: the engine in this process on Mesa, the shell against the stand-in channel or a script, PNGs on demand.
fn run_host(o: Options) -> io::Result<()> {
    // SAFETY: nothing else in this process reads the environment while it is set.
    unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    if let Some(ms) = o.clock {
        lumatv_ui::host::set_clock(ms);
    }
    let mut log = Log { file: o.log.as_ref().and_then(|p| std::fs::File::create(p).ok()) };
    std::fs::create_dir_all(&o.shm)?;
    let (width, height) = (1920u32, 1080u32);
    let gles = Gles::software_pbuffer(c"libEGL.so.1", width, height)?;
    log.line(&format!("GL: {}", gles.describe()));
    let (mut engine, app_end) = Engine::new(
        gles,
        HostConfig {
            width,
            height,
            refresh_hz: 60.0,
            shm_dir: o.shm.clone(),
            direct: false,
            gpu_sync: false,
            idle_trim: false,
            pages: 3,
            reference: o.reference,
            blit: true,
            external: false,
            system_z: i32::MAX,
        },
    )?;
    let (scene, _welcome, scene_events) = Scene::over_with(app_end, SceneOptions { shm_dir: Some(o.shm.clone()), ..SceneOptions::default() })?;
    engine.finish_handshake()?;
    let mut shell = make_shell(&o, scene)?;
    // a still of video (or Home) behind everything: what the system layer is drawn over
    let _backdrop = match &o.backdrop {
        Some(path) => {
            let (w, h, rgba) = read_png(path)?;
            let mut layer = shell.kit.ui.scene.image(w, h, 0, compositor_protocol::Props::at(0.0, 0.0))?;
            let pixels = layer.pixels_mut();
            for (dst, src) in pixels.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
                let a = u32::from(src[3]);
                dst[0] = (u32::from(src[0]) * a / 255) as u8;
                dst[1] = (u32::from(src[1]) * a / 255) as u8;
                dst[2] = (u32::from(src[2]) * a / 255) as u8;
                dst[3] = src[3];
            }
            layer.damage(None);
            Some(layer)
        }
        None => None,
    };
    let (tx, rx) = mpsc::channel::<Msg>();
    forward(scene_events, tx.clone(), Msg::Scene);
    let channel = match o.port {
        Some(port) => {
            let token = o.token_file.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.trim().to_string());
            let (feed, events) = Feed::start("channel", port, token, None)?;
            forward(events, tx.clone(), Msg::Channel);
            Some(feed)
        }
        None => None,
    };
    let helper = match o.port {
        Some(_) => {
            let (feed, events) = Feed::start_following("helper", o.helper_port, None, HELPER_TOPICS)?;
            forward(events, tx.clone(), Msg::Helper);
            Some(feed)
        }
        None => None,
    };
    carry(shell.start(), channel.as_ref(), helper.as_ref(), &mut log);

    let started = Instant::now();
    // A scripted step is given its own time (`t0 + ms`), not the clock's: on a loaded host a long frame lets several steps pile up and
    // run back to back, and a hold (OK down, then up 900 ms later by the script) must still be a hold to the models.
    let t0 = now_ms();
    let elapsed = |t0: u64| now_ms().saturating_sub(t0);
    let mut script: Vec<(u64, Step)> = Vec::new();
    for (ms, topic, doc) in o.pushes.clone() {
        script.push((ms, Box::new(move |s, _, _| s.channel(&Event::Topic { topic, data: doc }, t0 + ms))));
    }
    for (ms, tag, doc) in o.lunas.clone() {
        script.push((ms, Box::new(move |s, _, _| s.channel(&Event::Topic { topic: "luna".into(), data: json!({"tag": tag, "payload": doc}) }, t0 + ms))));
    }
    for (ms, topic, doc) in o.helpers.clone() {
        script.push((
            ms,
            Box::new(move |s, _, _| {
                let mut acts = s.helper(&Event::Up(true), t0 + ms);
                acts.extend(s.helper(&Event::Topic { topic, data: doc }, t0 + ms));
                acts
            }),
        ));
    }
    for (ms, what) in o.evals.clone() {
        script.push((ms, Box::new(move |s, _, log| {
            let v = match what.as_str() {
                "geometry" => s.geometry(),
                "lint" => s.lint(),
                "report" => s.report(),
                "volume" => s.debug(),
                "home" => s.home().state(),
                "atlas" => json!({"settled": s.kit.ui.glyph_text_settled(), "crowded": s.kit.ui.text_atlas_crowded(), "pictures": s.kit.ui.text_pictures, "fallbacks": s.kit.ui.glyph_text_fallbacks}),
                _ => s.state(),
            };
            log.line(&format!("[eval] {v}"));
            Vec::new()
        })));
    }
    for (ms, event) in o.keys.clone() {
        script.push((ms, Box::new(move |s, _, _| s.key(&event, t0 + ms))));
    }
    for (ms, path) in o.snaps.clone() {
        script.push((
            ms,
            Box::new(move |_, engine, log| {
                let rows = engine.read_window();
                match write_png(&path, width, height, &rows) {
                    Ok(()) => log.line(&format!("shot {}", path.display())),
                    Err(e) => log.line(&format!("shot {} failed: {e}", path.display())),
                }
                Vec::new()
            }),
        ));
    }
    script.sort_by_key(|(ms, _)| *ms);
    let mut frames_marks = o.frames.clone();
    frames_marks.sort_by_key(|(ms, _)| *ms);

    let mut drawn: u64 = 0;
    let mut gpu_total: u64 = 0;
    let mut gpu_max: u32 = 0;
    let mut px_total: u64 = 0;
    let mut mismatched: u64 = 0;
    let mut cpu_total: u64 = 0;
    let mut first_frame_ms: Option<u64> = None;
    let mut last_present_ns: u64 = 0;
    let mut pending_frame = true;
    while elapsed(t0) < o.wait_ms {
        let now = elapsed(t0);
        // the script's next step, the shell's timers and the next frame decide how long to wait
        let mut deadline = o.wait_ms.min(script.first().map_or(u64::MAX, |(ms, _)| *ms));
        if let Some(due) = shell.due_at() {
            deadline = deadline.min(due.saturating_sub(t0));
        }
        if let Some((ms, _)) = frames_marks.first() {
            deadline = deadline.min(*ms);
        }
        let demand = engine.demand();
        let animating = matches!(demand, Demand::EveryRefresh) || pending_frame || engine.has_commits() || engine.redraw_wanted();
        let timeout = if animating { Duration::from_millis(1) } else { Duration::from_millis(deadline.saturating_sub(now).clamp(1, 50)) };
        match rx.recv_timeout(timeout) {
            Ok(Msg::Scene(ev)) => shell.kit.ui.scene.observe(&ev),
            Ok(Msg::Channel(ev)) => carry(shell.channel(&ev, now_ms()), channel.as_ref(), helper.as_ref(), &mut log),
            Ok(Msg::Helper(ev)) => carry(shell.helper(&ev, now_ms()), channel.as_ref(), helper.as_ref(), &mut log),
            Ok(Msg::Key(ev)) => carry(shell.key(&ev, now_ms()), channel.as_ref(), helper.as_ref(), &mut log),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        while script.first().is_some_and(|(ms, _)| elapsed(t0) >= *ms) {
            let (_, step) = script.remove(0);
            let acts = step(&mut shell, &mut engine, &mut log);
            carry(acts, channel.as_ref(), helper.as_ref(), &mut log);
        }
        while frames_marks.first().is_some_and(|(ms, _)| elapsed(t0) >= *ms) {
            let (_, label) = frames_marks.remove(0);
            log.line(&format!("[frames] {label} {drawn}"));
        }
        if shell.due_at().is_some_and(|due| now_ms() >= due) {
            carry(shell.tick(now_ms()), channel.as_ref(), helper.as_ref(), &mut log);
        }
        if let Some(atlas) = shell.kit.ui.text_atlas() {
            atlas.pump();
        }
        if shell.kit.ui.scene.has_pending() {
            shell.kit.ui.try_commit()?;
        }
        if engine.pump()?.closed {
            return Err(io::Error::other("the scene closed"));
        }
        let now_ns = lumatv_render::now_ns();
        let due = match engine.demand() {
            Demand::EveryRefresh => true,
            Demand::At(at) => now_ns >= at,
            Demand::Idle => false,
        };
        if pending_frame || engine.has_commits() || engine.redraw_wanted() || due {
            pending_frame = false;
            let cost = engine.draw(now_ns);
            engine.gles().finish();
            // a window's swap waits for the next refresh: so does this, to the 60 Hz grid
            let next = last_present_ns + 16_666_667;
            if next > lumatv_render::now_ns() {
                std::thread::sleep(Duration::from_nanos(next - lumatv_render::now_ns()));
            }
            let presented = lumatv_render::now_ns();
            last_present_ns = presented;
            engine.frame_done(presented, &cost);
            drawn += 1;
            gpu_total += u64::from(cost.gpu_us);
            cpu_total += u64::from(cost.cpu_us);
            gpu_max = gpu_max.max(cost.gpu_us);
            px_total += cost.region_px;
            if cost.mismatch_px > 0 {
                mismatched += 1;
                log.line(&format!("[mismatch] frame {drawn} at {} ms: {} px differ from the frame drawn whole", elapsed(t0), cost.mismatch_px));
            }
            if first_frame_ms.is_none() && drawn > 1 {
                first_frame_ms = Some(elapsed(t0));
            }
            if o.log.is_some() {
                log.line(&format!(
                    "[frame] {drawn} at {} ms: damage {} px, drew {} px in {} rects ({:.1} %), layers {} culled {} cpu {} us gpu {} us{}",
                    elapsed(t0),
                    cost.damage_px,
                    cost.region_px,
                    cost.rects,
                    cost.region_px as f64 / 20736.0,
                    cost.layers,
                    cost.culled,
                    cost.cpu_us,
                    cost.gpu_us,
                    cost.gl_error.map_or(String::new(), |e| format!(" GL error {e:#x}"))
                ));
            }
            engine.tidy();
            shell.tidy(now_ns);
        } else {
            // Commits that show nothing by themselves (atlas chunks, layers made ahead) are applied without a frame, and the upload
            // work they leave is done now: `tidy_more` only says what the last tidy left over.
            let had = engine.has_waiting();
            engine.apply_waiting(now_ns);
            if had || engine.tidy_more() {
                engine.tidy();
            }
        }
    }
    log.line(&format!(
        "RESULT frames {drawn} in {:.1} s, gpu mean {} us max {} us, cpu mean {} us, drew {:.1} % of the panel per frame, text pictures {}, glyph fallbacks {}, frames differing from whole {mismatched}",
        started.elapsed().as_secs_f64(),
        gpu_total.checked_div(drawn).unwrap_or(0),
        gpu_max,
        cpu_total.checked_div(drawn).unwrap_or(0),
        (px_total as f64 / drawn.max(1) as f64) / 20736.0,
        shell.kit.ui.text_pictures,
        shell.kit.ui.glyph_text_fallbacks
    ));
    Ok(())
}
