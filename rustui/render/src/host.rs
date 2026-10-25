//! The compositor of one app, in the app's own process: LumaShow's session (which checks the app's requests), scene (which
//! animates layers on a clock) and renderer (which draws them on the GPU), driven by the host's loop instead of the
//! compositor's render and present threads.
//!
//! The app speaks to it as it speaks to LumaShow's compositor: `ui::Scene` over a socket (a socket pair here) with the same
//! requests, the same shared-memory files for pictures, the same events back (`Frame`, `AtlasUploaded`, `BackdropReady`,
//! `Motion`). Nothing of the toolkit knows the compositor is in the same process; the host calls [`Engine::pump`] to read
//! what the app sent, [`Engine::draw`] to draw a frame and [`Engine::frame_done`] to tell the app it was shown.
//!
//! Drawing. The renderer draws the way a LumaShow scan-out page is laid out: panel row 0 is the first row of the target's
//! memory, which is the top of the screen. A window surface is the other way up (GL row 0 is the bottom of the window), so
//! the scene is drawn into a render target of the panel's size and one more pass copies it into the window's framebuffer
//! turned over. [`HostConfig::direct`] skips the copy and draws the scene straight into the window's framebuffer, which then
//! shows upside down: for measuring what the copy costs.

use crate::clock::now_ns;
use crate::geometry::Panel;
use crate::gl::{Gles, RenderTarget};
use crate::idle::{Idle, Process};
use crate::ops::{Op, Transaction};
use crate::renderer::{Draw, DrawReport, Renderer, Surface};
use crate::scene::{Demand, Drawn, Scene};
use crate::session::{Config, Limits, Session};
use crate::stats::SharedStats;
use compositor_protocol::{Event, PROTOCOL_VERSION, Request, read_frame, write_frame};
use device_profile::Rotation;
use std::ffi::CStr;
use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How long housekeeping (freeing textures, uploading what the next frame will draw) may take in one go.
const TIDY_BUDGET: Duration = Duration::from_millis(2);

pub struct HostConfig {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: f32,
    /// Where the app's picture files live (`ui::SceneOptions::shm_dir`).
    pub shm_dir: PathBuf,
    /// Draw the scene straight into the window's framebuffer (it shows upside down), without the copy that turns it over.
    pub direct: bool,
    /// Wait for the GPU to finish each frame before `draw` returns, so that its time is in [`FrameCost::gpu_us`].
    pub gpu_sync: bool,
    /// Run the compositor's idle housekeeping (frees pooled render targets and heap pages after 5 s of quiet).
    pub idle_trim: bool,
}

/// What one frame cost the host.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameCost {
    /// Taking in the app's commits, placing and drawing the layers, and the copy into the window: the CPU time to issue the frame.
    pub cpu_us: u32,
    /// The same until the GPU had finished (`HostConfig::gpu_sync`), else 0.
    pub gpu_us: u32,
    /// Of `cpu_us`: placing the layers, planning backdrops, making backdrops, drawing.
    pub place_us: u32,
    pub plan_us: u32,
    pub backdrop_us: u32,
    pub draw_us: u32,
    pub layers: u32,
    pub culled: u32,
    pub backdrops: u16,
    pub uploaded_bytes: u32,
    pub draw_calls: u32,
    pub gl_error: Option<u32>,
}

/// What reading the app's requests did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pumped {
    /// Commits taken in (a frame is due for them).
    pub commits: u32,
    /// Requests that were not commits (changes staged for the next commit, or acted on at once).
    pub requests: u32,
    /// The app closed its end.
    pub closed: bool,
}

pub struct Engine {
    renderer: Renderer,
    scene: Scene,
    session: Session,
    stats: Arc<SharedStats>,
    stream: Option<UnixStream>,
    handshake: Option<JoinHandle<io::Result<UnixStream>>>,
    inbox: Vec<u8>,
    outbox: Vec<u8>,
    drawn: Vec<Drawn>,
    shown: Vec<Drawn>,
    target: RenderTarget,
    blit: Blit,
    window: (u32, u32),
    direct: bool,
    gpu_sync: bool,
    highest_tag: u64,
    taken_tag: u64,
    /// Ops taken in since the last frame, applied with the frame (as the render thread applies a wake's batches).
    waiting: Vec<Transaction>,
    idle: Option<Idle>,
    tidy_more: bool,
    uploaded_bytes: u32,
}

/// One pass that copies the scene's render target into the window's framebuffer, row order reversed.
struct Blit {
    program: u32,
    buffer: u32,
    ndc: i32,
    texel: i32,
    image: i32,
}

const BLIT_VERTEX: &CStr = c"#version 300 es
layout(location = 0) in vec2 a_panel;
layout(location = 1) in vec2 a_local;
uniform vec2 u_ndc;
out vec2 v_local;
void main() {
    v_local = a_local;
    gl_Position = vec4(a_panel * u_ndc - 1.0, 0.0, 1.0);
}";

const BLIT_FRAGMENT: &CStr = c"#version 300 es
precision mediump float;
in vec2 v_local;
uniform sampler2D u_image;
uniform vec2 u_texel;
out mediump vec4 o_color;
void main() {
    o_color = texture(u_image, v_local * u_texel);
}";

impl Blit {
    fn new(gl: &Gles, width: u32, height: u32) -> io::Result<Self> {
        let program = gl.program(BLIT_VERTEX, BLIT_FRAGMENT)?;
        gl.use_program(program);
        let ndc = gl.uniform_location(program, c"u_ndc");
        let texel = gl.uniform_location(program, c"u_texel");
        let image = gl.uniform_location(program, c"u_image");
        gl.uniform2f(ndc, 2.0 / width as f32, 2.0 / height as f32);
        gl.uniform2f(texel, 1.0 / width as f32, 1.0 / height as f32);
        gl.uniform1i(image, 0);
        let buffer = gl.new_vertex_buffer();
        let (w, h) = (width as f32, height as f32);
        // Window position (x, h - row) shows the target's pixel (x, row): the top of the screen is the target's row 0.
        gl.static_vertices(buffer, &[[0.0, h, 0.0, 0.0], [w, h, w, 0.0], [0.0, 0.0, 0.0, h], [w, 0.0, w, h]]);
        Ok(Self { program, buffer, ndc, texel, image })
    }

    fn run(&self, gl: &Gles, source: u32, width: u32, height: u32) {
        gl.bind_framebuffer(0);
        gl.viewport(0, 0, width as i32, height as i32);
        gl.disable(crate::gl::GL_SCISSOR_TEST);
        gl.disable(crate::gl::GL_BLEND);
        gl.use_program(self.program);
        gl.use_vertex_buffer(self.buffer);
        gl.active_texture(0);
        gl.bind_texture(source);
        gl.draw_triangle_strip(0, 4);
        let _ = (self.ndc, self.texel, self.image);
    }
}

impl Engine {
    /// A compositor for `gles` (the window's context, current on this thread) and the app's end of the socket it speaks over.
    /// The handshake (`Hello`, `Welcome`) is answered on a thread so that `ui::Scene::over_with`, which blocks on it, can run
    /// on this one; call [`Engine::finish_handshake`] once it has returned.
    pub fn new(gles: Gles, config: HostConfig) -> io::Result<(Self, UnixStream)> {
        let max_texture = gles.max_texture_size();
        let panel = Panel {
            rotation: Rotation::R0,
            logical_width: config.width as f32,
            logical_height: config.height as f32,
            width: config.width,
            height: config.height,
        };
        let target = gles.new_render_target(config.width, config.height)?;
        let blit = Blit::new(&gles, config.width, config.height)?;
        let mut renderer = Renderer::new(gles, panel)?;
        renderer.set_importer(None);
        if std::env::var("LUMASHOW_WARMUP").as_deref() != Ok("off") {
            let began = Instant::now();
            let error = renderer.warm_up();
            let _ = crate::gl::take_target_cost();
            crate::log::log(format!(
                "GPU warm-up: one frame of every program in {} ms{}",
                began.elapsed().as_millis(),
                error.map_or(String::new(), |e| format!(" (GL error {e:#x})"))
            ));
        }
        let stats = Arc::new(SharedStats::default());
        let session_config = Arc::new(Config {
            limits: Limits::for_texture_size(max_texture),
            panel_width: config.width,
            panel_height: config.height,
            refresh_hz: config.refresh_hz,
            shm_dir: config.shm_dir,
            gpu_opp_khz: Vec::new(),
        });
        let mut session = Session::new(session_config, Arc::clone(&stats), 1);
        // The app's Hello is answered by the thread below; the session is told of it here so that it is greeted.
        let _ = session.handle(Request::Hello { version: PROTOCOL_VERSION });
        let (app, ours) = UnixStream::pair()?;
        let (width, height, hz) = (config.width, config.height, config.refresh_hz);
        let handshake = thread::Builder::new().name("handshake".into()).spawn(move || {
            let mut ours = ours;
            let mut buf = Vec::new();
            match read_frame::<Request>(&mut ours, &mut buf)? {
                Some(Request::Hello { version }) if version == PROTOCOL_VERSION => {}
                other => return Err(io::Error::other(format!("expected Hello, got {other:?}"))),
            }
            write_frame(
                &mut ours,
                &Event::Welcome { version: PROTOCOL_VERSION, panel_width: width, panel_height: height, refresh_hz: hz, smooth_first_full: false },
            )?;
            Ok(ours)
        })?;
        let now = now_ns();
        Ok((
            Self {
                renderer,
                scene: Scene::new(panel.logical_width, panel.logical_height, (1e9 / f64::from(config.refresh_hz)) as u64),
                session,
                stats,
                stream: None,
                handshake: Some(handshake),
                inbox: Vec::new(),
                outbox: Vec::new(),
                drawn: Vec::new(),
                shown: Vec::new(),
                target,
                blit,
                window: (config.width, config.height),
                direct: config.direct,
                gpu_sync: config.gpu_sync,
                highest_tag: 0,
                taken_tag: 0,
                waiting: Vec::new(),
                idle: config.idle_trim.then(|| Idle::from_env(now)),
                tidy_more: false,
                uploaded_bytes: 0,
            },
            app,
        ))
    }

    pub fn finish_handshake(&mut self) -> io::Result<()> {
        let handle = self.handshake.take().ok_or_else(|| io::Error::other("handshake already finished"))?;
        let stream = handle.join().map_err(|_| io::Error::other("the handshake thread panicked"))??;
        stream.set_nonblocking(true)?;
        self.stream = Some(stream);
        Ok(())
    }

    pub fn gles(&self) -> &Gles {
        self.renderer.gles()
    }

    /// Reads what the app has sent, without waiting, runs it through the session and keeps the commits for the next frame.
    pub fn pump(&mut self) -> io::Result<Pumped> {
        let mut pumped = Pumped::default();
        let Some(stream) = self.stream.as_mut() else {
            return Ok(pumped);
        };
        let mut chunk = [0u8; 16 * 1024];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => {
                    pumped.closed = true;
                    break;
                }
                Ok(n) => self.inbox.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        let mut at = 0;
        let mut buf = Vec::new();
        loop {
            let rest = &self.inbox[at..];
            if rest.len() < 4 {
                break;
            }
            let len = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
            if rest.len() < 4 + len {
                break;
            }
            let mut frame = &rest[..4 + len];
            let request = read_frame::<Request>(&mut frame, &mut buf)?.ok_or_else(|| io::Error::other("an empty frame"))?;
            at += 4 + len;
            let is_commit = matches!(request, Request::Commit { .. });
            let outcome = self.session.handle(request);
            for event in outcome.replies {
                self.send(&event);
            }
            if let Some(mut transaction) = outcome.commit {
                transaction.received_ns = now_ns();
                if let Some(tag) = transaction.tag {
                    self.highest_tag = self.highest_tag.max(tag);
                }
                self.waiting.push(transaction);
            }
            if is_commit {
                pumped.commits += 1;
            } else {
                pumped.requests += 1;
            }
            if outcome.close {
                pumped.closed = true;
            }
        }
        self.inbox.drain(..at);
        self.flush();
        Ok(pumped)
    }

    fn send(&mut self, event: &Event) {
        let _ = write_frame(&mut self.outbox, event);
    }

    /// Writes the events queued for the app. A full socket keeps the rest for the next call: the app reads its events on a thread
    /// of its own, so this clears at once in practice.
    fn flush(&mut self) {
        let Some(stream) = self.stream.as_mut() else {
            return;
        };
        let mut sent = 0;
        while sent < self.outbox.len() {
            match io::Write::write(stream, &self.outbox[sent..]) {
                Ok(0) => break,
                Ok(n) => sent += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        self.outbox.drain(..sent);
    }

    /// Commits are waiting for a frame.
    pub fn has_commits(&self) -> bool {
        self.waiting.iter().any(Transaction::asks_for_frame)
    }

    /// What the scene wants on its own: an animation at every refresh, a wallpaper's step at its time, or nothing.
    pub fn demand(&self) -> Demand {
        self.scene.demand()
    }

    /// Work between frames changed what the next frame draws (a glyph list swapped in, a backdrop published).
    pub fn redraw_wanted(&self) -> bool {
        self.renderer.wants_frame()
    }

    /// Whether housekeeping has more to do.
    pub fn tidy_more(&self) -> bool {
        self.tidy_more
    }

    /// Housekeeping for at most a couple of milliseconds: the work a frame leaves undone (textures to free and to upload, a chunk
    /// of an atlas, a prepared backdrop). Call it while the GPU has the frame, or when nothing is due.
    pub fn tidy(&mut self) {
        let mut done = self.renderer.tidy(TIDY_BUDGET);
        if !done.more {
            let made = self.renderer.prepare(&self.scene, TIDY_BUDGET);
            done.captured += made.captured;
            done.more = made.more;
        }
        self.uploaded_bytes = self.uploaded_bytes.saturating_add(u32::try_from(done.uploaded_bytes).unwrap_or(u32::MAX));
        self.tidy_more = done.more;
        self.forward_events();
    }

    /// The quiet moments (`idle.rs`): after a few seconds of nothing, frees what a still screen does not need.
    pub fn idle_moment(&mut self, busy: bool) {
        let now = now_ns();
        if let Some(idle) = self.idle.as_mut() {
            if busy {
                idle.active(now);
            } else {
                idle.moment(&mut self.renderer, &mut Process, now, true);
            }
        }
    }

    /// When the idle housekeeping next wants the loop to wake.
    pub fn idle_wake_at(&self, ready: bool) -> Option<u64> {
        self.idle.as_ref().and_then(|idle| idle.wake_at(ready))
    }

    fn forward_events(&mut self) {
        for event in self.renderer.drain_events() {
            self.send(&event);
        }
        let mut notices = Vec::new();
        self.scene.take_notices(&mut notices);
        for (id, notice) in notices {
            self.send(&Event::Motion {
                id,
                generation: notice.generation,
                kind: notice.kind,
                value: notice.value,
                velocity: notice.velocity,
                time_ns: notice.time_ns,
            });
        }
        self.flush();
    }

    /// Applies the commits that wait to the scene and the renderer, in order, with no frame for them: what the app staged that
    /// shows nothing by itself (an atlas chunk, a layer made ahead). `draw` does the same for the commits a frame is due for.
    pub fn apply_waiting(&mut self, now_ns: u64) {
        for batch in std::mem::take(&mut self.waiting) {
            if let Some(tag) = batch.tag {
                self.taken_tag = self.taken_tag.max(tag);
            }
            for op in &batch.ops {
                self.scene.apply(op, now_ns);
                self.renderer.apply(op);
            }
        }
        self.forward_events();
    }

    /// Takes in the waiting commits, draws the scene as it is at `now_ns` and leaves it in the window's framebuffer, ready for
    /// the swap.
    pub fn draw(&mut self, now_ns: u64) -> FrameCost {
        let began = Instant::now();
        self.apply_waiting(now_ns);
        self.scene.advance(now_ns);
        self.scene.resolve(now_ns, &mut self.drawn);
        self.renderer.set_time(now_ns);
        let target = if self.direct { 0 } else { self.target.framebuffer };
        let draws = [Draw { surface: Surface { stack: 0, from: 0, to: self.drawn.len(), transparent: false }, slot: None, target }];
        let report: DrawReport = self.renderer.draw_frame(&self.drawn, &draws, &self.scene).remove(0);
        if !self.direct {
            self.blit.run(self.renderer.gles(), self.target.texture, self.window.0, self.window.1);
        }
        self.shown.clone_from(&self.drawn);
        self.renderer.set_shown(&self.shown);
        self.scene.observe(self.renderer.unseen());
        self.scene.fade(report.fading, true);
        let cpu_us = began.elapsed().as_micros() as u32;
        let gpu_us = if self.gpu_sync {
            self.renderer.gles().finish();
            began.elapsed().as_micros() as u32
        } else {
            0
        };
        let (_captures, uploaded) = self.renderer.take_frame_counts();
        let calls = report.gl;
        FrameCost {
            cpu_us,
            gpu_us,
            place_us: (report.place_ns / 1000) as u32,
            plan_us: (report.plan_ns / 1000) as u32,
            backdrop_us: (report.backdrop_ns / 1000) as u32,
            draw_us: (report.draw_ns / 1000) as u32,
            layers: report.drawn,
            culled: report.culled,
            backdrops: report.backdrops,
            uploaded_bytes: uploaded.saturating_add(self.uploaded_bytes),
            draw_calls: calls.draws,
            gl_error: report.gl_error,
        }
    }

    /// The frame drawn last reached the screen at `presented_ns`: tells the app (which retires what the commit replaced).
    pub fn frame_done(&mut self, presented_ns: u64, cost: &FrameCost) {
        self.stats.frame(cost.cpu_us.max(cost.gpu_us), 0);
        self.uploaded_bytes = 0;
        self.send(&Event::Frame {
            tag: self.taken_tag,
            presented_ns,
            render_us: cost.cpu_us.max(cost.gpu_us),
            missed: 0,
            backdrops: cost.backdrops,
            gpu_mhz: 0,
            captures: 0,
            uploaded_bytes: cost.uploaded_bytes,
            quality: 0,
        });
        self.forward_events();
    }

    /// The window's framebuffer as RGBA rows from the bottom (for a screenshot on a host without a window system).
    pub fn read_window(&self) -> Vec<u8> {
        let gl = self.renderer.gles();
        gl.bind_framebuffer(0);
        gl.read_pixels(0, 0, self.window.0, self.window.1)
    }

    pub fn layers(&self) -> usize {
        self.scene.len()
    }
}
