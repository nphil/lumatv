//! `lumatv-rustui`: LumaHome drawn with LumaShow's Rust UI toolkit, as a client of LG's compositor (or, on a host, into a
//! pbuffer). The phase 3 spike of `docs/research/rustui-result.md`.
//!
//!   lumatv-rustui [--mode window|offscreen] [--run idle:60,mix:30] [--assets DIR] [--shm DIR] [--log FILE]
//!                 [--shot FILE] [--direct] [--gpu-sync] [--no-trim] [--pages N] [--verify] [--keys script|evdev|both] [--libmali PATH]
//!
//! `--run` is a list of `phase:seconds`; a phase is `idle`, `dpad`, `sweep` or `mix` (dpad and sweep in turn), named as in the QML
//! gate. Each phase ends with a summary line; the whole run ends with one more.

mod input;
mod metrics;
mod output;
#[cfg(feature = "window")]
mod window;

use compositor_protocol::Event;
use lumatv_render::host::{Engine, HostConfig};
use lumatv_render::{Demand, now_ns};
use lumatv_rustui::home::{self, Home, Key};
use lumatv_rustui::script;
use metrics::{Frame, Log, Marker, Phase};
use output::Output;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use ui::{SceneOptions, Ui};

struct Options {
    mode: String,
    run: Vec<(String, f64)>,
    assets: PathBuf,
    shm: PathBuf,
    log: Option<String>,
    shot: Option<String>,
    frames: Option<String>,
    direct: bool,
    gpu_sync: bool,
    trim: bool,
    pages: usize,
    verify: bool,
    keys: String,
    libmali: Option<String>,
    window_type: String,
}

fn parse() -> Result<Options, String> {
    let mut o = Options {
        mode: "offscreen".into(),
        run: vec![("idle".into(), 5.0), ("mix".into(), 12.0)],
        assets: PathBuf::from("payload"),
        shm: std::env::temp_dir().join("lumatv-rustui-shm"),
        log: None,
        shot: None,
        frames: None,
        direct: false,
        gpu_sync: false,
        trim: true,
        pages: 0,
        verify: false,
        keys: "script".into(),
        libmali: std::env::var("LUMATV_GFX_LIB").ok(),
        window_type: "_WEBOS_WINDOW_TYPE_OVERLAY".into(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--mode" => o.mode = value("--mode")?,
            "--run" => {
                o.run = value("--run")?
                    .split(',')
                    .map(|p| {
                        let (n, s) = p.split_once(':').ok_or_else(|| format!("{p}: want phase:seconds"))?;
                        Ok((n.to_owned(), s.parse::<f64>().map_err(|e| format!("{p}: {e}"))?))
                    })
                    .collect::<Result<_, String>>()?
            }
            "--assets" => o.assets = PathBuf::from(value("--assets")?),
            "--shm" => o.shm = PathBuf::from(value("--shm")?),
            "--log" => o.log = Some(value("--log")?),
            "--shot" => o.shot = Some(value("--shot")?),
            "--frames" => o.frames = Some(value("--frames")?),
            "--direct" => o.direct = true,
            "--gpu-sync" => o.gpu_sync = true,
            "--no-trim" => o.trim = false,
            "--pages" => o.pages = value("--pages")?.parse().map_err(|e| format!("--pages: {e}"))?,
            "--verify" => o.verify = true,
            "--keys" => o.keys = value("--keys")?,
            "--libmali" => o.libmali = Some(value("--libmali")?),
            "--window-type" => o.window_type = value("--window-type")?,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(o)
}

fn main() {
    let options = match parse() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(64);
        }
    };
    if let Err(e) = run(options) {
        eprintln!("lumatv-rustui: {e}");
        std::process::exit(1);
    }
}

fn run(o: Options) -> io::Result<()> {
    let mut log = Log::open(o.log.as_deref());
    let path = std::env::var("LUMATV_GFX_PATH").unwrap_or_else(|_| "host".into());
    log.line(&format!("start: mode {} gfx path {} lib {:?}", o.mode, path, o.libmali));
    std::fs::create_dir_all(&o.shm)?;
    let (gles, mut out): (_, Box<dyn Output>) = match o.mode.as_str() {
        "offscreen" => output::offscreen(home::W as u32, home::H as u32)?,
        #[cfg(feature = "window")]
        "window" => window::open(
            home::W as u32,
            home::H as u32,
            o.libmali.as_deref().ok_or_else(|| io::Error::other("--libmali (or LUMATV_GFX_LIB) names the Mali driver"))?,
            &o.window_type,
            matches!(o.keys.as_str(), "wayland" | "both"),
        )?,
        other => return Err(io::Error::other(format!("mode {other} is not built in"))),
    };
    log.line(&format!("GL: {}", gles.describe()));
    let (mut engine, app_end) = Engine::new(
        gles,
        HostConfig {
            width: home::W as u32,
            height: home::H as u32,
            refresh_hz: 60.0,
            shm_dir: o.shm.clone(),
            direct: o.direct,
            gpu_sync: o.gpu_sync,
            idle_trim: o.trim,
            pages: o.pages,
            reference: o.verify,
            blit: true,
            external: false,
            system_z: i32::MAX,
        },
    )?;
    let (scene, _welcome, events) = ui::Scene::over_with(app_end, SceneOptions { shm_dir: Some(o.shm.clone()), ..SceneOptions::default() })?;
    engine.finish_handshake()?;
    let mut ui = Ui::new(scene);
    ui.enable_glyph_text().map_err(|e| io::Error::other(format!("{e}")))?;
    let built = std::time::Instant::now();
    let landed = std::time::Instant::now();
    let mut land = |ui: &mut Ui| -> io::Result<()> {
        let began = std::time::Instant::now();
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
    let mut home = Home::build(ui, &o.assets, &mut land)?;
    land(&mut home.ui)?;
    log.line(&format!(
        "built the home in {} ms (atlas landed); text pictures {}, glyph text fallbacks {}",
        built.elapsed().as_millis(),
        home.ui.text_pictures,
        home.ui.glyph_text_fallbacks
    ));
    let _ = landed;

    let mut evdev = if matches!(o.keys.as_str(), "evdev" | "both") { Some(input::Evdev::open()) } else { None };
    let compositor_pid = metrics::pid_of("surface-manager");
    let tck = metrics::clk_tck();
    let mut phases: Vec<Phase> = Vec::new();
    let total_started = now_ns();
    let mut marker = Marker::open();

    // First frame: whatever the build and the atlas staged, then the run.
    let mut last_present_ns = 0u64;
    let mut pending_frame = true;
    let mut drawn_frames: u64 = 0;

    for (name, seconds) in o.run.clone() {
        let presses = script::build(&name, seconds);
        let mut next_press = 0usize;
        let mut phase = Phase { name: name.clone(), started_ns: now_ns(), clk_tck: tck, rss_kb_start: metrics::rss_kb(), ..Phase::default() };
        marker.write(&format!("P:{name}"));
        let cpu0 = metrics::proc_cpu("self").unwrap_or(0);
        let comp0 = compositor_pid.as_deref().and_then(metrics::proc_cpu);
        let gpu0 = metrics::gpu_counters();
        let end_ns = phase.started_ns + (seconds * 1e9) as u64;
        let mut second_mark = phase.started_ns + 1_000_000_000;
        let mut second_frames = 0u32;
        let mut second_worst = 0.0f32;
        let mut keys_down: Vec<Key> = Vec::new();

        loop {
            let now = now_ns();
            if now >= end_ns {
                break;
            }
            // what the compositor and the remote said
            drain_events(&events, &mut home);
            if let Some(atlas) = home.ui.text_atlas() {
                atlas.pump();
            }
            keys_down.clear();
            keys_down.extend(out.dispatch()?);
            if let Some(e) = evdev.as_mut() {
                keys_down.extend(e.poll());
            }
            while next_press < presses.len() && phase.started_ns + (presses[next_press].at_s * 1e9) as u64 <= now {
                keys_down.push(presses[next_press].key);
                next_press += 1;
            }
            for k in keys_down.drain(..) {
                home.key(k, now);
                phase.keys += 1;
            }
            let home_wake = home.tick(now);
            if home.ui.scene.has_pending() {
                let _ = home.ui.try_commit()?;
            }
            let pumped = engine.pump()?;
            if pumped.closed {
                return Err(io::Error::other("the compositor side of the toolkit closed"));
            }

            // is a frame due?
            let demand = engine.demand();
            let due = match demand {
                Demand::EveryRefresh => true,
                Demand::At(slot) => now >= slot,
                Demand::Idle => false,
            };
            if pending_frame || engine.has_commits() || engine.redraw_wanted() || due {
                pending_frame = false;
                let started = now_ns();
                let cost = engine.draw(started);
                if let Some(code) = cost.gl_error {
                    log.line(&format!("GL error {code:#x}"));
                }
                if cost.mismatch_px > 0 {
                    log.line(&format!(
                        "MISMATCH frame {drawn_frames}: {} px differ from the same frame drawn whole, first {:?}",
                        cost.mismatch_px,
                        engine.mismatch()
                    ));
                }
                if std::env::var_os("LUMATV_TRACE_FRAMES").is_some() {
                    log.line(&format!(
                        "frame {drawn_frames}: damage {} px, drew {} px in {} rects ({:.1} %), layers {} culled {} cpu {} us gpu {} us",
                        cost.damage_px,
                        cost.region_px,
                        cost.rects,
                        cost.region_px as f64 / 20736.0,
                        cost.layers,
                        cost.culled,
                        cost.cpu_us,
                        cost.gpu_us
                    ));
                }
                let swap_began = now_ns();
                out.present(&mut engine)?;
                let presented = now_ns();
                engine.frame_done(presented, &cost);
                drawn_frames += 1;
                let gap_ms = if last_present_ns == 0 { 0.0 } else { (presented - last_present_ns) as f32 / 1e6 };
                last_present_ns = presented;
                phase.frames.push(Frame {
                    gap_ms,
                    cpu_us: cost.cpu_us,
                    gpu_us: cost.gpu_us,
                    swap_us: ((presented - swap_began) / 1000) as u32,
                    animated: demand == Demand::EveryRefresh,
                });
                second_frames += 1;
                second_worst = second_worst.max(gap_ms);
                engine.tidy();
                engine.idle_moment(true);
            } else {
                // Nothing is due: housekeeping while there is any, then sleep until the next thing that is.
                engine.apply_waiting(now);
                if engine.tidy_more() {
                    engine.tidy();
                    continue;
                }
                engine.idle_moment(false);
                let mut deadline = end_ns;
                if let Some(w) = home_wake {
                    deadline = deadline.min(w);
                }
                if let Demand::At(slot) = demand {
                    deadline = deadline.min(slot);
                }
                if next_press < presses.len() {
                    deadline = deadline.min(phase.started_ns + (presses[next_press].at_s * 1e9) as u64);
                }
                if let Some(w) = engine.idle_wake_at(demand == Demand::Idle) {
                    deadline = deadline.min(w);
                }
                // a commit the app made meanwhile is read at the top of the loop
                out.wait(Some(deadline.min(second_mark.max(now + 1_000_000))))?;
            }

            if now_ns() >= second_mark {
                log.line(&format!(
                    "{name}: {second_frames} frames this second, worst gap {second_worst:.1} ms, rss {} kB, focus {}",
                    metrics::rss_kb(),
                    home.focus_label()
                ));
                second_mark += 1_000_000_000;
                second_frames = 0;
                second_worst = 0.0;
            }
        }
        phase.ended_ns = now_ns();
        phase.cpu_ticks = metrics::proc_cpu("self").unwrap_or(0).saturating_sub(cpu0);
        phase.compositor_ticks = match (compositor_pid.as_deref().and_then(metrics::proc_cpu), comp0) {
            (Some(a), Some(b)) => Some(a.saturating_sub(b)),
            _ => None,
        };
        phase.gpu_busy = match (metrics::gpu_counters(), gpu0) {
            (Some(a), Some(b)) => Some((a.0.saturating_sub(b.0), a.1.saturating_sub(b.1))),
            _ => None,
        };
        phase.rss_kb_end = metrics::rss_kb();
        phase.rss_kb_max = phase.rss_kb_end.max(phase.rss_kb_start);
        log.line(&format!("RESULT {}", phase.summary()));
        if let Some(path) = &o.frames {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = f.write_all(phase.csv().as_bytes());
            }
        }
        phases.push(phase);
    }
    marker.write("P:end");
    log.line(&format!(
        "done: {} frames in {:.1} s, layers {}, text runs {}",
        drawn_frames,
        (now_ns() - total_started) as f64 / 1e9,
        engine.layers(),
        if home.ui.text_pictures == 0 { "all glyph quads" } else { "some pictures" }
    ));
    if let Some(shot) = &o.shot {
        out.screenshot(&mut engine, shot)?;
        log.line(&format!("screenshot {shot}"));
    }
    Ok(())
}

fn drain_events(events: &std::sync::mpsc::Receiver<Event>, home: &mut Home) {
    while let Ok(event) = events.try_recv() {
        home.ui.scene.observe(&event);
    }
}
