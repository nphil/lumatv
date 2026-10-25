//! The renderer's host and LumaHome in one process on Mesa's software GL, stepped on a clock the caller keeps: what the tests and the
//! benchmark run. The loop is `main.rs`'s, without the window, the sleeping and the real time: a step is a stretch of the rig's own
//! clock, in which keys arrive, the app ticks, and a frame is drawn if the scene wants one.

use crate::home::{Home, Key};
use crate::script;
use compositor_protocol::Event;
use lumatv_render::host::{Engine, FrameCost, HostConfig};
use lumatv_render::{Demand, Gles, now_ns};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Duration;
use ui::{SceneOptions, Ui};

/// One refresh of a 60 Hz panel.
pub const REFRESH_NS: u64 = 16_666_667;

#[derive(Clone, Copy, Debug)]
pub struct RigConfig {
    /// Render targets frames go into in turn (`HostConfig::pages`).
    pub pages: usize,
    /// Draw each frame whole as well and compare (`HostConfig::reference`).
    pub reference: bool,
    /// Wait for the GPU after each frame (`HostConfig::gpu_sync`): what the frame's time is measured by.
    pub gpu_sync: bool,
}

pub struct Rig {
    pub engine: Engine,
    pub home: Home,
    events: Receiver<Event>,
    /// The rig's clock (`CLOCK_MONOTONIC` nanoseconds), moved by [`Rig::step`].
    pub now: u64,
    began: u64,
    pending: bool,
    shm: PathBuf,
}

/// Mesa picks the GPU if the machine has one; the tests and the benchmark are about the software rasteriser, which is the same on
/// every machine.
pub fn software_gl() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: runs once, before the first context is made; nothing else in the process reads the environment concurrently.
        unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    });
}

/// `payload/`, whose theme and wallpaper picture LumaHome reads.
pub fn assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../payload")
}

impl Rig {
    pub fn new(config: RigConfig) -> io::Result<Self> {
        Self::with_assets(config, &assets())
    }

    pub fn with_assets(config: RigConfig, assets: &Path) -> io::Result<Self> {
        software_gl();
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let shm = std::env::temp_dir().join(format!("lumatv-rig-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&shm)?;
        let (width, height) = (crate::home::W as u32, crate::home::H as u32);
        let gles = Gles::software_pbuffer(c"libEGL.so.1", width, height)?;
        let (mut engine, app_end) = Engine::new(
            gles,
            HostConfig {
                width,
                height,
                refresh_hz: 60.0,
                shm_dir: shm.clone(),
                direct: false,
                gpu_sync: config.gpu_sync,
                idle_trim: false,
                pages: config.pages,
                reference: config.reference,
                blit: false,
                external: false,
                system_z: i32::MAX,
            },
        )?;
        let (scene, _welcome, events) = ui::Scene::over_with(app_end, SceneOptions { shm_dir: Some(shm.clone()), ..SceneOptions::default() })?;
        engine.finish_handshake()?;
        let mut ui = Ui::new(scene);
        ui.enable_glyph_text().map_err(|e| io::Error::other(format!("{e}")))?;
        // Until the glyphs have landed a label is a picture for good: the build waits for them, as `main.rs` does.
        let land = |ui: &mut Ui, engine: &mut Engine, events: &Receiver<Event>| -> io::Result<()> {
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
                if began.elapsed() > Duration::from_secs(10) {
                    return Err(io::Error::other("the glyph atlas did not land in 10 s"));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        };
        let mut home = Home::build(ui, assets, &mut |ui| land(ui, &mut engine, &events))?;
        land(&mut home.ui, &mut engine, &events)?;
        let now = now_ns();
        let mut rig = Self { engine, home, events, now, began: now, pending: true, shm };
        rig.home.set_clock(Some(rig.wall()));
        rig.home.tick_clock(true);
        Ok(rig)
    }

    /// The time of day the app shows: 10:09:58 when the rig began, then the rig's own seconds.
    fn wall(&self) -> (i32, i32, i32) {
        let seconds = 10 * 3600 + 9 * 60 + 58 + ((self.now - self.began) / 1_000_000_000) as i32;
        (seconds / 3600 % 24, seconds / 60 % 60, seconds % 60)
    }

    /// Moves the rig's clock on by `dt_ns` and does what the app's loop does in that time: the keys arrive, the app ticks (the
    /// clock's hands, the lift), what it committed is taken in, and a frame is drawn if anything wants one. `None` when nothing did.
    pub fn step(&mut self, dt_ns: u64, keys: &[Key]) -> io::Result<Option<FrameCost>> {
        self.now += dt_ns;
        let now = self.now;
        while let Ok(event) = self.events.try_recv() {
            self.home.ui.scene.observe(&event);
        }
        if let Some(atlas) = self.home.ui.text_atlas() {
            atlas.pump();
        }
        for &key in keys {
            self.home.key(key, now);
        }
        self.home.set_clock(Some(self.wall()));
        self.home.tick(now);
        if self.home.ui.scene.has_pending() {
            let _ = self.home.ui.try_commit()?;
        }
        if self.engine.pump()?.closed {
            return Err(io::Error::other("the engine's side of the toolkit closed"));
        }
        let due = match self.engine.demand() {
            Demand::EveryRefresh => true,
            Demand::At(slot) => now >= slot,
            Demand::Idle => false,
        };
        if self.pending || self.engine.has_commits() || self.engine.redraw_wanted() || due {
            self.pending = false;
            let cost = self.engine.draw(now);
            self.engine.frame_done(now, &cost);
            self.engine.tidy();
            return Ok(Some(cost));
        }
        self.engine.apply_waiting(now);
        if self.engine.tidy_more() {
            self.engine.tidy();
        }
        Ok(None)
    }

    /// Runs a named script of the gate (`idle`, `dpad`, `sweep`, `mix`) for `seconds` of the rig's clock at 60 steps a second, and
    /// hands each frame's cost to `each`. Returns the frames drawn.
    pub fn run(&mut self, name: &str, seconds: f64, mut each: impl FnMut(&FrameCost)) -> io::Result<usize> {
        let presses = script::build(name, seconds);
        let start = self.now;
        let mut next = 0;
        let mut frames = 0;
        let mut keys = Vec::new();
        while self.now - start < (seconds * 1e9) as u64 {
            let at = self.now + REFRESH_NS - start;
            keys.clear();
            while next < presses.len() && (presses[next].at_s * 1e9) as u64 <= at {
                keys.push(presses[next].key);
                next += 1;
            }
            if let Some(cost) = self.step(REFRESH_NS, &keys)? {
                each(&cost);
                frames += 1;
            }
        }
        Ok(frames)
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.shm);
    }
}

/// The host's engine and a bare toolkit scene, for tests that make their own layers: `frame` commits what they staged and draws it.
pub struct Stage {
    pub engine: Engine,
    pub ui: Ui,
    events: Receiver<Event>,
    /// The stage's clock (`CLOCK_MONOTONIC` nanoseconds), one refresh further on for every frame.
    pub now: u64,
    shm: PathBuf,
}

impl Stage {
    pub fn new(config: RigConfig) -> io::Result<Self> {
        software_gl();
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let shm = std::env::temp_dir().join(format!("lumatv-stage-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&shm)?;
        let (width, height) = (crate::home::W as u32, crate::home::H as u32);
        let gles = Gles::software_pbuffer(c"libEGL.so.1", width, height)?;
        let (mut engine, app_end) = Engine::new(
            gles,
            HostConfig {
                width,
                height,
                refresh_hz: 60.0,
                shm_dir: shm.clone(),
                direct: false,
                gpu_sync: config.gpu_sync,
                idle_trim: false,
                pages: config.pages,
                reference: config.reference,
                blit: false,
                external: false,
                system_z: i32::MAX,
            },
        )?;
        let (scene, _welcome, events) = ui::Scene::over_with(app_end, SceneOptions { shm_dir: Some(shm.clone()), ..SceneOptions::default() })?;
        engine.finish_handshake()?;
        Ok(Self { engine, ui: Ui::new(scene), events, now: now_ns(), shm })
    }

    /// One refresh on: what the app staged is committed and taken in, and the frame is drawn.
    pub fn frame(&mut self) -> io::Result<FrameCost> {
        self.now += REFRESH_NS;
        while let Ok(event) = self.events.try_recv() {
            self.ui.scene.observe(&event);
        }
        if self.ui.scene.has_pending() {
            let _ = self.ui.try_commit()?;
        }
        if self.engine.pump()?.closed {
            return Err(io::Error::other("the engine's side of the toolkit closed"));
        }
        let cost = self.engine.draw(self.now);
        self.engine.frame_done(self.now, &cost);
        self.engine.tidy();
        Ok(cost)
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.shm);
    }
}
