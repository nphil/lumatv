//! A small surface for the engine to draw: a ground and three cards of glass under the windows (stack 0), a ticker that crosses a
//! dock, and a volume bar over the windows (stack 1, the system layer). It is what `lumatv-ui` is, in miniature: the app that owns
//! the layers, in this process, over the socket the engine gave it.

use compositor_protocol::{Backdrop, Ease, Event, Fill, Glass, Props, Rect, Rgba, Shadow, Shape, Sheen};
use lumashow_ui::{GlassLayer, SceneOptions, ShapeLayer, Ui as Toolkit};
use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::Receiver;

/// Where the system layer begins: the volume bar's layers.
pub const SYSTEM_Z: i32 = 1000;

const TRACK: (f32, f32, f32, f32) = (760.0, 936.0, 400.0, 12.0);

fn solid(radius: f32, colour: Rgba) -> Shape {
    Shape { corner_radius: radius, fill: Fill::Solid(colour), border_width: 0.0, border: Rgba(0, 0, 0, 0) }
}

fn card(backdrop: Backdrop, tint: Rgba) -> Glass {
    Glass {
        corner_radius: 24.0,
        tint,
        edge: Rgba(30, 30, 30, 30),
        edge_width: 1.0,
        highlight: Rgba(12, 12, 12, 12),
        sheen: Some(Sheen { peak: 0.16, floor: 0.05, start: 0.38, end: 0.6 }),
        shadows: [Some(Shadow { offset_y: 12.0, blur: 30.0, colour: Rgba(0, 0, 0, 70) }), None],
        depth: None,
        backdrop,
        saturation: 1.2,
        refraction: 0.0,
    }
}

pub struct Demo {
    ui: Toolkit,
    events: Receiver<Event>,
    ticker: ShapeLayer,
    seconds: ShapeLayer,
    pill: GlassLayer,
    track: ShapeLayer,
    fill: ShapeLayer,
    /// The cards, kept so that they stay on the screen.
    _cards: Vec<GlassLayer>,
    _ground: ShapeLayer,
    /// The volume bar is up with this level (0 to 1).
    level: Option<f32>,
    options: Options,
    /// Where the ticker and the seconds bar were last put, so that a tick that moves nothing commits nothing.
    ticker_x: f32,
    steps: f32,
}

/// How the demo paces itself.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// How many times a second the ticker moves (a commit each time): 60 is an animation, 10 the idle Home's wallpaper step.
    pub ticker_hz: u32,
    /// Every this many seconds the volume bar comes up for 2 s (the system layer over the windows); `None` keeps it down.
    pub volume_period_s: Option<u32>,
}

impl Default for Options {
    fn default() -> Self {
        Options { ticker_hz: 60, volume_period_s: None }
    }
}

impl Options {
    /// `LUMATV_UI_DEMO_HZ` and `LUMATV_UI_DEMO_VOLUME_S` from the environment, the defaults where unset or unreadable.
    pub fn from_env() -> Self {
        let read = |name: &str| std::env::var(name).ok().and_then(|v| v.trim().parse::<u32>().ok());
        Options { ticker_hz: read("LUMATV_UI_DEMO_HZ").unwrap_or(60), volume_period_s: read("LUMATV_UI_DEMO_VOLUME_S") }
    }
}

impl Demo {
    /// The layers, committed over `app_end` (the end of the socket [`super::Ui::new`] returned); the engine must answer the
    /// handshake meanwhile ([`super::Ui::finish_handshake`] after this returns).
    pub fn new(app_end: UnixStream, shm_dir: &Path) -> io::Result<Self> {
        Self::with(app_end, shm_dir, Options::default())
    }

    pub fn with(app_end: UnixStream, shm_dir: &Path, options: Options) -> io::Result<Self> {
        let (scene, _welcome, events) = lumashow_ui::Scene::over_with(app_end, SceneOptions { shm_dir: Some(shm_dir.to_owned()), ..SceneOptions::default() })?;
        let ui = Toolkit::new(scene);
        let s = &ui.scene;
        let ground = s.shape(1920, 1080, 0, solid(0.0, Rgba(18, 20, 32, 255)), Props::at(0.0, 0.0));
        let tint = Rgba(12, 10, 20, 100);
        let cards = vec![
            s.glass(560, 300, 5, card(Backdrop::Cached { radius: 30.0 }, tint), Props::at(96.0, 96.0)),
            s.glass(560, 300, 5, card(Backdrop::Cached { radius: 30.0 }, tint), Props::at(688.0, 96.0)),
            s.glass(544, 300, 5, card(Backdrop::Cached { radius: 30.0 }, tint), Props::at(1280.0, 96.0)),
            s.glass(1728, 280, 5, card(Backdrop::Cached { radius: 30.0 }, tint), Props::at(96.0, 700.0)),
        ];
        let ticker = s.shape(120, 12, 6, solid(6.0, Rgba(167, 155, 245, 255)), Props::at(130.0, 830.0));
        // a bar that grows a step a second: its clip is as wide as the seconds so far
        let seconds = s.shape(
            400,
            10,
            6,
            solid(5.0, Rgba(200, 200, 220, 255)),
            Props { clip: Some(Rect { x: 130.0, y: 360.0, w: 0.0, h: 10.0 }), ..Props::at(130.0, 360.0) },
        );
        // the volume bar: a plate, a track and the level as a clip of a bar, hidden until it is asked for
        let hidden = |x: f32, y: f32| Props { opacity: 0.0, visible: false, ..Props::at(x, y) };
        let pill = s.glass(520, 84, SYSTEM_Z, card(Backdrop::None, Rgba(14, 12, 24, 235)), hidden(700.0, 900.0));
        let track = s.shape(TRACK.2 as u32, TRACK.3 as u32, SYSTEM_Z + 1, solid(6.0, Rgba(60, 60, 78, 255)), hidden(TRACK.0, TRACK.1));
        let fill = s.shape(TRACK.2 as u32, TRACK.3 as u32, SYSTEM_Z + 2, solid(6.0, Rgba(245, 245, 255, 255)), hidden(TRACK.0, TRACK.1));
        Ok(Self { ui, events, ticker, seconds, pill, track, fill, _cards: cards, _ground: ground, level: None, options, ticker_x: f32::NAN, steps: f32::NAN })
    }

    /// What the app does on its clock: the ticker moves (`Options::ticker_hz` times a second), the seconds bar grows a step on a
    /// new second, and the volume bar comes and goes on its cycle. Commits what it staged, if anything.
    pub fn tick(&mut self, now_ns: u64) -> io::Result<()> {
        while let Ok(event) = self.events.try_recv() {
            self.ui.scene.observe(&event);
        }
        let ms = now_ns / 1_000_000;
        let step_ms = (1000 / u64::from(self.options.ticker_hz.max(1))).max(1);
        let x = 130.0 + ((ms / step_ms * step_ms) % 1500) as f32;
        if x != self.ticker_x {
            self.ticker_x = x;
            self.ticker.set(Props::at(x, 830.0));
        }
        let steps = ((ms / 1000) % 20) as f32;
        if steps != self.steps {
            self.steps = steps;
            self.seconds.set(Props { clip: Some(Rect { x: 130.0, y: 360.0, w: steps * 20.0, h: 10.0 }), ..Props::at(130.0, 360.0) });
        }
        if let Some(period_s) = self.options.volume_period_s {
            // up for the first 2 s of every period, at a level that changes each time
            let period_ms = u64::from(period_s.max(3)) * 1000;
            let up = ms % period_ms < 2000;
            if up != self.level.is_some() {
                if up {
                    self.show_volume([0.4, 0.55, 0.8][(ms / period_ms % 3) as usize]);
                } else {
                    self.hide_volume();
                }
            }
        }
        if self.ui.scene.has_pending() {
            let _ = self.ui.try_commit()?;
        }
        Ok(())
    }

    /// Puts the volume bar up at `level` (0 to 1) over the windows, fading in if it was down.
    pub fn show_volume(&mut self, level: f32) {
        let level = level.clamp(0.0, 1.0);
        let fade = |at: (f32, f32)| Props::at(at.0, at.1);
        if self.level.is_none() {
            for (layer, at) in [(&*self.pill, (700.0, 900.0)), (&*self.track, (TRACK.0, TRACK.1))] {
                layer.set(Props { opacity: 0.0, ..fade(at) });
                layer.animate(fade(at), 120, Ease::Out);
            }
            self.fill.set(Props { opacity: 0.0, ..fade((TRACK.0, TRACK.1)) });
            self.fill.animate(Props { clip: Some(bar(level)), ..fade((TRACK.0, TRACK.1)) }, 120, Ease::Out);
        } else {
            self.fill.set(Props { clip: Some(bar(level)), ..fade((TRACK.0, TRACK.1)) });
        }
        self.level = Some(level);
    }

    /// Takes the volume bar down.
    pub fn hide_volume(&mut self) {
        let at = |x: f32, y: f32| Props { opacity: 0.0, visible: false, ..Props::at(x, y) };
        self.pill.set(at(700.0, 900.0));
        self.track.set(at(TRACK.0, TRACK.1));
        self.fill.set(at(TRACK.0, TRACK.1));
        self.level = None;
    }
}

/// The rectangle of the fill that shows at `level`.
fn bar(level: f32) -> Rect {
    Rect { x: TRACK.0, y: TRACK.1, w: TRACK.2 * level, h: TRACK.3 }
}
