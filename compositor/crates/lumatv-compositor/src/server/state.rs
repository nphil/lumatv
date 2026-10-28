//! The compositor's state: everything the protocol handlers mutate, the globals, and the frame tick.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use wayland_backend::server::{ClientData, ClientId, DisconnectReason};
use wayland_server::protocol::wl_buffer::WlBuffer;
use wayland_server::protocol::wl_callback::WlCallback;
use wayland_server::protocol::wl_compositor::WlCompositor;
use wayland_server::protocol::wl_data_device_manager::WlDataDeviceManager;
use wayland_server::protocol::wl_output::WlOutput;
use wayland_server::protocol::wl_seat::WlSeat;
use wayland_server::protocol::wl_shell::WlShell;
use wayland_server::protocol::wl_shm::WlShm;
use wayland_server::protocol::wl_subcompositor::WlSubcompositor;
use wayland_server::{Client, DisplayHandle, Resource};

use lumatv_wl_protocols::input_method::server::{input_method::InputMethod, input_panel::InputPanel};
use lumatv_wl_protocols::mali_buffer_sharing::server::mali_buffer_sharing::MaliBufferSharing;
use lumatv_wl_protocols::starfish::server::{wl_starfish_output::WlStarfishOutput, wl_starfish_pointer::WlStarfishPointer};
use lumatv_wl_protocols::text::server::text_model_factory::TextModelFactory;
use lumatv_wl_protocols::webos_extension::server::wl_webos_xinput_extension::WlWebosXinputExtension;
use lumatv_wl_protocols::webos_foreign::server::wl_webos_foreign::WlWebosForeign;
use lumatv_wl_protocols::webos_input_manager::server::wl_webos_input_manager::WlWebosInputManager;
use lumatv_wl_protocols::webos_shell::server::wl_webos_shell::WlWebosShell;
use lumatv_wl_protocols::webos_surface_group::server::wl_webos_surface_group_compositor::WlWebosSurfaceGroupCompositor;
use lumatv_wl_protocols::webos_system_ux::server::wl_webos_system_ux::WlWebosSystemUx;
use wayland_protocols::wp::linux_dmabuf::zv1::server::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1;

use super::foreign::ForeignState;
use super::seat::SeatState;
use crate::capture;
use crate::config::{Check, Config};
use crate::luna;
use crate::output::{Output, Presented};
use crate::render::gl::BufferPath;
use crate::scene::Scene;
use crate::surface::{SurfaceId, Surfaces};

#[derive(Default, Debug, Clone, Copy)]
pub struct Stats {
    pub requests: u64,
    pub commits: u64,
    pub buffers_created: u64,
    pub ticks: u64,
    pub frames_drawn: u64,
    pub callbacks_sent: u64,
    /// Of those, the shell client's that went out before their frame landed ([`Owed::ahead`]).
    pub callbacks_ahead: u64,
    pub releases: u64,
    pub clients_total: u64,
    pub render_us_last: u64,
    pub render_us_max: u64,
    pub skipped_items: u64,
    pub gl_errors: u64,
    /// Buffers whose first-use costs the commit paid (`Output::prepare_buffer`).
    pub prepared: u64,
    /// Buffers a frame had to import itself (zero while every commit prepares its buffer).
    pub frame_imports: u64,
    /// Ticks that drew nothing because the driver was still busy with a buffer a commit had prepared.
    pub held_ticks: u64,
}

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub pid: i32,
    pub uid: u32,
    pub since: Instant,
    /// The process's command line, read at connect while the process is alive (a client that only binds globals
    /// and leaves is otherwise anonymous: the 7 s client of the 2026-10-25 gate).
    pub cmd: String,
    /// Globals bound, in order: what a client that leaves without a surface was looking for.
    pub bound: Vec<&'static str>,
}

impl ClientInfo {
    fn new(pid: i32, uid: u32) -> Self {
        ClientInfo { pid, uid, since: Instant::now(), cmd: if pid > 0 { cmdline(pid) } else { String::new() }, bound: Vec::new() }
    }
}

/// `/proc/PID/cmdline`, arguments joined by spaces, cut at 160 bytes.
fn cmdline(pid: i32) -> String {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    let mut s: String = String::from_utf8_lossy(&raw).replace('\0', " ").trim_end().to_string();
    if s.len() > 160 {
        let mut cut = 160;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
    }
    s
}

/// The X2 self-check: counts drawn frames whose probe pixels match the pattern.
#[derive(Default, Debug, Clone, Copy)]
pub struct CheckState {
    pub next_frame: u32,
    pub ok: u64,
    pub bad: u64,
    pub unread: u64,
}

/// What a frame on its way to the screen owes once it is there.
struct Owed {
    /// Frame callbacks, fired when the frame lands.
    callbacks: Vec<WlCallback>,
    /// The shell client's frame callbacks, fired as soon as the frame is the oldest on its way (at its draw when none is ahead of it), a
    /// frame before it lands. The shell's Qt starts each frame on the callback of the one before: 2-3.5 ms of CPU, then 11-13.5 ms of
    /// GPU on the picker's slide, and a flip needs the GPU done before its refresh. Started at the landing the frame had less than a
    /// millisecond to spare, and 5 of 34 slide frames took two refreshes (round 4: the quick panel, lighter by a millisecond or two,
    /// took none). Started a frame ahead the next frame is made while this one waits for its refresh.
    ahead: Vec<WlCallback>,
    /// What the shell window's commits replaced: released when the frame lands instead of when it is drawn, since a client let run ahead
    /// takes the first buffer it is given back, and the one on the screen until this frame lands must not be drawn into.
    buffers: Vec<WlBuffer>,
}

pub struct State {
    pub dh: DisplayHandle,
    pub config: Config,
    pub surfaces: Surfaces,
    pub scene: Scene,
    pub output: Box<dyn Output>,
    pub seat: SeatState,
    pub foreign: ForeignState,
    pub luna: Box<dyn luna::Bridge>,
    pub luna_state: crate::luna_methods::LunaState,
    pub keys: crate::input::Keys,
    pub ime: super::ime::ImeState,
    /// The shell client and its UI channel (`shell.rs`).
    pub shell: crate::shell::Shell,
    /// The app switcher (`switcher.rs`).
    pub switcher: crate::switcher::Switcher,
    /// The owner's switch for the transitions (`transition.rs`): the theme file, and `ctl transitions` over it.
    pub motion: crate::transition::ThemeSwitch,
    pub stats: Stats,
    pub check: CheckState,
    pub clients: HashMap<ClientId, ClientInfo>,
    /// Recent protocol-level events worth showing on the control socket (bounded).
    pub journal: Vec<String>,
    pub should_exit: bool,
    pub last_capture: Option<PathBuf>,
    /// Captured frames are encoded and written on its thread.
    pub pngs: capture::Writer,
    /// The `wl_webos_shell_surface` of each window that has one.
    pub webos_shells: HashMap<crate::window::WindowId, lumatv_wl_protocols::webos_shell::server::wl_webos_shell_surface::WlWebosShellSurface>,
    pub started: Instant,
    /// Whether the shell client's frame callbacks go out ahead of their flip ([`Owed::ahead`]); `ctl callbacks shell landing` turns it
    /// off, at the flip like any client's, for an A/B on the set.
    pub shell_ahead: bool,
    serial: u32,
    /// Buffers released at the end of the tick that presented the frame which stopped reading them.
    release_after_present: Vec<WlBuffer>,
    /// What frames on their way to the screen owe, oldest first, paid when each lands ([`State::frames_landed`]). Frame callbacks go
    /// out at the flip: firing at the tick paced clients to the latch, which the pacer moves by a few milliseconds whenever a
    /// slow frame enters or leaves its window; at the flip they follow the refresh. The shell client's go ahead of it ([`Owed::ahead`]).
    frames_in_flight: std::collections::VecDeque<(Instant, Owed)>,
    /// Per surface: when its last new buffer came and how many came in a row within 20 ms of each other, to log
    /// the gaps of a client that was keeping up (what the X1 gate counts, seen from this side).
    cadence: HashMap<SurfaceId, (Instant, u32)>,
    /// The last tick drew nothing because a driver buffer was warming (one log line per such run).
    holding: bool,
}

impl State {
    pub fn new(dh: DisplayHandle, config: Config, output: Box<dyn Output>, luna: Box<dyn luna::Bridge>) -> State {
        let keys = crate::input::Keys::new(&config);
        let mut scene = Scene::default();
        scene.care = crate::oledcare::Care::new(&config.data_dir);
        let motion = crate::transition::ThemeSwitch::new(&config.data_dir);
        State {
            dh,
            config,
            surfaces: Surfaces::default(),
            scene,
            output,
            seat: SeatState { seats: 1, ..SeatState::default() },
            foreign: ForeignState::default(),
            luna,
            luna_state: crate::luna_methods::LunaState::default(),
            keys,
            ime: super::ime::ImeState::default(),
            shell: crate::shell::Shell::default(),
            switcher: crate::switcher::Switcher::default(),
            motion,
            stats: Stats::default(),
            check: CheckState::default(),
            clients: HashMap::new(),
            journal: Vec::new(),
            should_exit: false,
            last_capture: None,
            pngs: capture::Writer::default(),
            webos_shells: HashMap::new(),
            started: Instant::now(),
            shell_ahead: true,
            serial: 0,
            release_after_present: Vec::new(),
            cadence: HashMap::new(),
            frames_in_flight: std::collections::VecDeque::new(),
            holding: false,
        }
    }

    /// Registers every global the compositor serves. `mali_buffer_sharing` only when this compositor serves the
    /// buffers itself (`--buffers own`, or no GL at all so a client's bind at least gets an answer).
    pub fn create_globals(dh: &DisplayHandle, config: &Config, own_mali: bool) {
        dh.create_global::<State, WlCompositor, ()>(4, ());
        dh.create_global::<State, WlSubcompositor, ()>(1, ());
        dh.create_global::<State, WlShm, ()>(1, ());
        dh.create_global::<State, WlShell, ()>(1, ());
        dh.create_global::<State, WlSeat, super::seat::SeatId>(4, super::seat::SeatId(0));
        dh.create_global::<State, WlOutput, ()>(2, ());
        dh.create_global::<State, WlDataDeviceManager, ()>(1, ());
        dh.create_global::<State, WlWebosShell, ()>(1, ());
        dh.create_global::<State, WlWebosSurfaceGroupCompositor, ()>(1, ());
        dh.create_global::<State, WlWebosForeign, ()>(1, ());
        dh.create_global::<State, WlWebosInputManager, ()>(1, ());
        dh.create_global::<State, WlWebosXinputExtension, ()>(1, ());
        dh.create_global::<State, TextModelFactory, ()>(1, ());
        dh.create_global::<State, InputMethod, ()>(2, ());
        dh.create_global::<State, InputPanel, ()>(1, ());
        dh.create_global::<State, WlWebosSystemUx, ()>(1, ());
        dh.create_global::<State, ZwpLinuxDmabufV1, ()>(3, ());
        if own_mali {
            dh.create_global::<State, MaliBufferSharing, ()>(5, ());
        }
        if config.starfish {
            dh.create_global::<State, WlStarfishPointer, ()>(1, ());
            dh.create_global::<State, WlStarfishOutput, ()>(1, ());
        }
    }

    /// One more `wl_seat` global, for an input device (`create_globals` made seat 0); returns its id, which the
    /// device's keys are sent on.
    pub fn add_seat(&mut self, device: &str) -> super::seat::SeatId {
        let id = super::seat::SeatId(self.seat.seats);
        self.seat.seats += 1;
        self.dh.create_global::<State, WlSeat, super::seat::SeatId>(4, id);
        log::info!("{}: {device}", id.name());
        id
    }

    pub fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1);
        self.serial
    }

    pub fn note_request(&mut self, iface: &str, id: u32, what: &str) {
        self.stats.requests += 1;
        if self.config.log_protocol {
            log::debug!("{iface}@{id} {what}");
        }
    }

    pub fn journal(&mut self, line: String) {
        log::info!("{line}");
        if self.journal.len() >= 200 {
            self.journal.remove(0);
        }
        self.journal.push(line);
    }

    pub fn client_info(&self, client: &Client) -> String {
        match self.clients.get(&client.id()) {
            Some(c) => format!("pid {}", c.pid),
            None => "client ?".into(),
        }
    }

    /// `wl_surface.commit` and what follows it: apply state, learn buffer sizes, map or unmap windows.
    pub fn commit(&mut self, id: SurfaceId) {
        self.stats.commits += 1;
        let State { surfaces, output, .. } = self;
        let effects = surfaces.commit(id, |b| match crate::buffer::size_of(b) {
            Some(size) => Some((size, true)),
            None => output.query_buffer(b),
        });
        for (sid, eff) in effects {
            if eff.new_buffer
                && let Some(s) = self.surfaces.get(sid)
                && s.current.size.is_none()
            {
                log::warn!("surface {} committed a buffer of unknown size", sid.0);
            }
            if let Some(mapped) = eff.mapped_changed {
                self.scene.set_mapped(sid, mapped);
                if let Some(wid) = self.scene.window_of(sid) {
                    let w = self.scene.window(wid).unwrap();
                    let is_card = w.kind.is_card();
                    let line = format!(
                        "window {} {} ({:?} app {:?} title {:?} size {:?})",
                        wid.0,
                        if mapped { "mapped" } else { "unmapped" },
                        w.kind,
                        w.app_id(),
                        w.title,
                        eff.size
                    );
                    self.journal(line);
                    if mapped {
                        super::webos_shell::on_mapped(self, wid);
                    } else if is_card {
                        super::webos_shell::on_card_gone(self);
                    }
                    super::seat::refocus(self);
                }
            }
            // The keyboard's window came, went, changed size or changed its input region: the field that holds the activation is told.
            if self.scene.window_of(sid).and_then(|w| self.scene.window(w)).is_some_and(|w| w.kind == crate::window::WindowType::Keyboard) {
                super::ime::refresh_panel(self);
            }
            if eff.new_buffer {
                if let Some(w) = self.scene.window_of(sid).and_then(|w| self.scene.window_mut(w)) {
                    w.commits += 1;
                    w.last_commit_ns = crate::output::monotonic_ns();
                }
                self.note_cadence(sid);
                // The buffer's first-use costs are paid here, in the clients' pass, not in the frame that first
                // draws it (the driver's import of a client buffer cost X1's first second a refresh on three gates).
                if let Some(b) = self.surfaces.get(sid).and_then(|s| s.current.buffer.clone())
                    && self.output.prepare_buffer(&b)
                {
                    self.stats.prepared += 1;
                }
            }
        }
        // A window that commits but never attaches a buffer never maps (HDMI 2 on the 2026-10-25 gate: created,
        // fullscreen, exported, never mapped); the first such commit is journalled once per window.
        if let Some(wid) = self.scene.window_of(id)
            && !self.surfaces.get(id).map(|s| s.is_mapped()).unwrap_or(false)
            && self.scene.window(wid).map(|w| !w.bare_commit_seen).unwrap_or(false)
        {
            let w = self.scene.window_mut(wid).unwrap();
            w.bare_commit_seen = true;
            let line = format!("window {} committed without a buffer ({:?} app {:?} state {:?})", wid.0, w.kind, w.app_id(), w.state);
            self.journal(line);
        }
    }

    fn note_cadence(&mut self, sid: SurfaceId) {
        let now = Instant::now();
        let (last, steady) = self.cadence.entry(sid).or_insert((now, 0));
        let gap = now.duration_since(*last);
        if gap <= Duration::from_millis(20) {
            *steady += 1;
        } else {
            // After at least half a second at the refresh rate, a gap under 100 ms is a stall, not a pause.
            if *steady >= 30 && gap < Duration::from_millis(100) {
                let app = self.scene.window_of(sid).and_then(|w| self.scene.window(w)).map(|w| w.app_id().to_string()).unwrap_or_default();
                log::warn!("frame gap {:.1} ms on surface {} ({app}) after {} steady frames", gap.as_secs_f64() * 1000.0, sid.0, *steady);
            }
            *steady = 0;
        }
        *last = now;
    }

    pub fn surface_destroyed(&mut self, id: SurfaceId) {
        // A surface that goes away with content no frame has shown is drawn once first: LG's libmali client
        // disconnects about a millisecond after its last swap, well inside the 16.7 ms tick, and its last frame
        // would otherwise never reach the output (first light re-run 2026-10-25: 6 commits, 5 drawn). Not while the
        // output paces a run of frames: there the extra frame would take the refresh the paced one is aimed at, and
        // the regular tick is at most a refresh away.
        if self.surfaces.has_undrawn(id) && self.scene.window_of(id).is_some() && self.output.next_frame_at(Instant::now()).is_none() {
            self.tick();
        }
        self.scene.zombify(id, &self.surfaces);
        if let Some(s) = self.surfaces.remove(id) {
            for b in s.current.buffer.into_iter().chain(s.current.retired) {
                self.release_after_present.push(b);
            }
            for cb in s.current.frame_callbacks {
                // A callback on a destroyed surface is never fired; dropping the object destroys it client-side.
                drop(cb);
            }
        }
        let was_card = self.scene.window_of(id).and_then(|wid| self.scene.window(wid)).is_some_and(|w| w.kind.is_card());
        self.scene.remove_surface(id);
        super::ime::surface_gone(self, id);
        self.cadence.remove(&id);
        self.foreign.surface_destroyed(id);
        if was_card {
            super::webos_shell::on_card_gone(self);
        }
        super::seat::refocus(self);
    }

    /// One frame: plan, draw what changed, present, pay out frame callbacks and buffer releases. A tick over half a
    /// refresh logs what each phase took, so a stall the loop's line attributes to `tick` is read without guessing.
    /// While the driver is busy with a buffer a commit just prepared (`Output::warming`) nothing is drawn: the previous
    /// frame stays on the screen, commits and callbacks keep their place, and the next tick tries again.
    pub fn tick(&mut self) -> Presented {
        self.tick_at(crate::output::monotonic_ns())
    }

    /// [`State::tick`] with the scene planned at `now_ns` (CLOCK_MONOTONIC): what the transitions' poses are evaluated at (the host tests move it by hand).
    pub fn tick_at(&mut self, now_ns: u64) -> Presented {
        let t0 = Instant::now();
        self.stats.ticks += 1;
        // A flip that landed while the pass did other work goes out before this frame is drawn, and one that landed
        // while it was drawn goes out before the tick's own bookkeeping: its callbacks start the clients' next frames.
        // X1's 20.4 ms frame gap on the eighth gate came with a flip the loop reached 3.6 ms after it landed.
        self.land_flips();
        if self.output.warming() {
            self.stats.held_ticks += 1;
            if !self.holding {
                self.holding = true;
                log::info!("tick: frame held while a driver buffer warms");
            }
            return Presented::default();
        }
        if self.holding {
            self.holding = false;
            log::info!("tick: warm done, drawing again");
        }
        self.scene.cursor =
            if self.seat.cursor_visible { Some(crate::render::cursor::rect_at(self.seat.pointer_position.0, self.seat.pointer_position.1)) } else { None };
        self.switcher_frame(t0);
        let State { scene, surfaces, output, stats, shell, .. } = self;
        scene.care.refresh(now_ns);
        scene.care.saver = shell.ui_state.get("saver").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let plan = scene.plan_at(surfaces, now_ns);
        let mut drew = false;
        let ui_changed = output.ui_needs_frame().unwrap_or_else(|e| {
            log::error!("system UI: {e}");
            false
        });
        if !plan.damage.is_empty() || ui_changed {
            match output.render(plan, surfaces) {
                Ok(st) => {
                    drew = true;
                    stats.render_us_last = st.render_us;
                    stats.render_us_max = stats.render_us_max.max(st.render_us);
                    stats.skipped_items += st.skipped as u64;
                    stats.gl_errors = st.gl_errors;
                    stats.frame_imports += st.imports as u64;
                }
                Err(e) => log::error!("render: {e}"),
            }
        }
        let t_rendered = Instant::now();
        if drew && self.scene.care_frame() {
            self.output.quiet_frame();
        }
        let presented = match self.output.present() {
            Ok(p) => p,
            Err(e) => {
                log::error!("present: {e}");
                Presented::default()
            }
        };
        let t_presented = Instant::now();
        self.land_flips();
        if drew {
            self.stats.frames_drawn += 1;
            self.after_drawn_frame();
        }
        let State { scene, surfaces, shell_ahead, .. } = &mut *self;
        let ahead = |id: SurfaceId| *shell_ahead && scene.window_of(id).and_then(|w| scene.window(w)).is_some_and(|w| w.kind.is_shell());
        let (callbacks, ahead_callbacks) = surfaces.take_frame_callbacks(ahead);
        let (retired, held) = surfaces.take_retired(ahead);
        self.route_callbacks(Owed { callbacks, ahead: ahead_callbacks, buffers: held }, presented);
        let t_callbacks = Instant::now();
        for b in self.release_after_present.drain(..).chain(retired) {
            if b.is_alive() {
                b.release();
                self.stats.releases += 1;
            }
        }
        self.surfaces.clear_damage();
        let t_released = Instant::now();
        if drew {
            self.luna_notify();
        }
        if let Some(n) = self.config.frames
            && self.stats.frames_drawn >= n
        {
            self.should_exit = true;
        }
        let t_end = Instant::now();
        if t_end - t0 > Duration::from_millis(8) {
            log::warn!(
                "tick: {:.1} ms: plan and draw {:.1}, present {:.1}, callbacks {:.1}, releases {:.1}, luna {:.1}",
                (t_end - t0).as_secs_f64() * 1000.0,
                (t_rendered - t0).as_secs_f64() * 1000.0,
                (t_presented - t_rendered).as_secs_f64() * 1000.0,
                (t_callbacks - t_presented).as_secs_f64() * 1000.0,
                (t_released - t_callbacks).as_secs_f64() * 1000.0,
                (t_end - t_released).as_secs_f64() * 1000.0
            );
        }
        presented
    }

    /// What a tick's frame owes goes out when it lands: its frame callbacks and the buffers it stopped reading. A tick that put
    /// nothing new on the way joins the last frame in flight (a client's callbacks never overtake its earlier ones), or pays at
    /// once when none is. The oldest frame in flight has its shell callbacks fired at once ([`Owed::ahead`]).
    fn route_callbacks(&mut self, owed: Owed, presented: Presented) {
        // A flip that never reports (the output off, a lost event) must not hold clients forever.
        while self.frames_in_flight.front().is_some_and(|(t, _)| t.elapsed() > Duration::from_millis(250)) {
            let (_, old) = self.frames_in_flight.pop_front().unwrap();
            log::warn!("frame callbacks held 250 ms without a flip; sent anyway");
            self.pay(old, presented.at_ms);
        }
        if presented.pending && !presented.replaced {
            self.frames_in_flight.push_back((Instant::now(), owed));
        } else if let Some((_, last)) = self.frames_in_flight.back_mut() {
            last.callbacks.extend(owed.callbacks);
            last.ahead.extend(owed.ahead);
            last.buffers.extend(owed.buffers);
        } else {
            self.pay(owed, presented.at_ms);
        }
        self.fire_ahead(presented.at_ms);
    }

    /// `n` frames reached the screen (the output's page-flip events): their callbacks are due, and are sent now rather
    /// than with the rest of the pass.
    pub fn frames_landed(&mut self, n: u32) {
        let mut fired = false;
        for _ in 0..n {
            let Some((_, owed)) = self.frames_in_flight.pop_front() else { break };
            self.pay(owed, crate::output::monotonic_ms());
            fired = true;
        }
        if fired {
            self.fire_ahead(crate::output::monotonic_ms());
            if let Err(e) = self.dh.flush_clients() {
                log::warn!("flush after a flip: {e}");
            }
        }
    }

    /// Takes the page flips that have landed (none is fine: the output does not wait) and sends their callbacks.
    pub fn land_flips(&mut self) {
        match self.output.dispatch_events() {
            Ok(n) => self.frames_landed(n),
            Err(e) => log::warn!("output events: {e}"),
        }
    }

    fn fire_callbacks(&mut self, callbacks: Vec<WlCallback>, at_ms: u32) {
        for cb in callbacks {
            if cb.is_alive() {
                cb.done(at_ms);
                self.stats.callbacks_sent += 1;
            }
        }
    }

    /// A frame is on the screen, or none is on its way: its callbacks are due and what it stopped reading is free.
    fn pay(&mut self, owed: Owed, at_ms: u32) {
        self.fire_callbacks(owed.ahead, at_ms);
        self.fire_callbacks(owed.callbacks, at_ms);
        self.release(owed.buffers);
    }

    /// The oldest frame in flight has no frame ahead of it: its shell callbacks go out now, so the client's next frame is made
    /// while this one waits for its refresh.
    fn fire_ahead(&mut self, at_ms: u32) {
        let Some((_, oldest)) = self.frames_in_flight.front_mut() else { return };
        let ahead = std::mem::take(&mut oldest.ahead);
        self.stats.callbacks_ahead += ahead.len() as u64;
        self.fire_callbacks(ahead, at_ms);
    }

    fn release(&mut self, buffers: Vec<WlBuffer>) {
        for b in buffers {
            if b.is_alive() {
                b.release();
                self.stats.releases += 1;
            }
        }
    }

    fn after_drawn_frame(&mut self) {
        let want_capture = self.config.capture_dir.is_some() && self.stats.frames_drawn % self.config.capture_every == 0;
        if !want_capture && self.config.check == Check::None {
            return;
        }
        let image = match self.output.capture() {
            Ok(i) => i,
            Err(e) => {
                log::warn!("capture: {e}");
                self.check.unread += 1;
                return;
            }
        };
        if self.config.check == Check::X2 {
            let k = self.check.next_frame;
            let expected = capture::x2_colour(k);
            let probe = self
                .scene
                .last_plan()
                .items
                .first()
                .map(|i| (i.rect.x.max(0) as u32, i.rect.y.max(0) as u32, i.rect.w as u32, i.rect.h as u32))
                .unwrap_or((0, 0, image.width, image.height));
            let ok = capture::matches(&image, probe, expected);
            let px = image.pixel(probe.0 + probe.2 / 2, probe.1 + probe.3 / 2);
            self.journal(format!(
                "check x2 frame {k}: centre {:02x}{:02x}{:02x}{:02x} expected {:02x}{:02x}{:02x}{:02x} {}",
                px[0],
                px[1],
                px[2],
                px[3],
                expected[0],
                expected[1],
                expected[2],
                expected[3],
                if ok { "ok" } else { "BAD" }
            ));
            if ok {
                self.check.ok += 1;
            } else {
                self.check.bad += 1;
            }
            self.check.next_frame += 1;
        }
        if want_capture && let Some(dir) = &self.config.capture_dir {
            let path = dir.join(format!("frame-{:04}.png", self.stats.frames_drawn));
            if self.pngs.write(path.clone(), image) {
                self.last_capture = Some(path);
            }
        }
    }

    /// Reads the current output back, for a picture written off the loop (the control socket's `capture`).
    pub fn capture_image(&mut self) -> std::io::Result<crate::output::Image> {
        self.output.capture()
    }

    /// One line per field, for the control socket and the exit summary.
    pub fn summary(&self) -> String {
        let s = &self.stats;
        let out = self.output.stats_line();
        format!(
            "uptime {:.1}s clients {} (total {}) surfaces {} windows {} requests {} commits {} buffers {} ticks {} drawn {} callbacks {} releases {} skipped {} gl errors {} render last {} us max {} us prepared {} frame imports {} held ticks {} check ok {} bad {} unread {}{}{}",
            self.started.elapsed().as_secs_f64(),
            self.clients.len(),
            s.clients_total,
            self.surfaces.len(),
            self.scene.windows().count(),
            s.requests,
            s.commits,
            s.buffers_created,
            s.ticks,
            s.frames_drawn,
            s.callbacks_sent,
            s.releases,
            s.skipped_items,
            s.gl_errors,
            s.render_us_last,
            s.render_us_max,
            s.prepared,
            s.frame_imports,
            s.held_ticks,
            self.check.ok,
            self.check.bad,
            self.check.unread,
            if out.is_empty() { "" } else { " " },
            out
        )
    }

    /// Whether a commit is waiting to be drawn and the output would rather draw it now than at the tick.
    pub fn wants_frame_now(&self) -> bool {
        self.surfaces.any_undrawn() && self.output.draws_on_damage(Instant::now())
    }

    /// The system UI app's socket is readable (`Output::ui_fd`): its requests are taken in now, so that a commit does not wait
    /// for the tick. Whether a frame should be drawn at once: the UI wants one and the output shows a still frame, as for a
    /// window's commit ([`State::wants_frame_now`]); otherwise the tick draws it.
    pub fn ui_wake(&mut self) -> bool {
        match self.output.ui_needs_frame() {
            Ok(wants) => wants && self.output.draws_on_damage(Instant::now()),
            Err(e) => {
                log::error!("system UI: {e}");
                false
            }
        }
    }

    pub fn windows_report(&self) -> Vec<serde_json::Value> {
        self.scene
            .windows()
            .map(|w| {
                let s = self.surfaces.get(w.surface);
                serde_json::json!({
                    "id": w.id.0,
                    "surface": w.surface.0,
                    "type": format!("{:?}", w.kind),
                    "state": format!("{:?}", w.state),
                    "title": w.title,
                    "appId": w.app_id(),
                    "mapped": s.map(|s| s.is_mapped()).unwrap_or(false),
                    "size": s.and_then(|s| s.current.size),
                    "position": w.position,
                    "keyMask": w.key_mask,
                    "inGroup": w.in_group,
                    "front": w.front,
                    "exposed": w.exposed,
                    "props": w.props,
                })
            })
            .collect()
    }

    pub fn buffer_path(&self) -> BufferPath {
        self.config.buffers
    }

    /// Outbound luna calls so far (what the bridge would have sent).
    pub fn luna_sent(&self) -> Vec<(String, serde_json::Value)> {
        self.luna.sent()
    }
}

/// Per-client bookkeeping; the backend calls these from `dispatch_clients`.
pub struct ClientHandler {
    pub events: Arc<Mutex<Vec<ClientEvent>>>,
}

#[derive(Debug, Clone)]
pub enum ClientEvent {
    Connected(ClientId),
    Disconnected(ClientId, String),
}

impl ClientData for ClientHandler {
    fn initialized(&self, client_id: ClientId) {
        self.events.lock().push(ClientEvent::Connected(client_id));
    }

    fn disconnected(&self, client_id: ClientId, reason: DisconnectReason) {
        self.events.lock().push(ClientEvent::Disconnected(client_id, format!("{reason:?}")));
    }
}

impl State {
    /// Applies connect/disconnect events the backend queued.
    pub fn drain_client_events(&mut self, events: &Arc<Mutex<Vec<ClientEvent>>>) {
        let evs: Vec<ClientEvent> = std::mem::take(&mut *events.lock());
        for ev in evs {
            match ev {
                ClientEvent::Connected(id) => self.client_connected(id),
                ClientEvent::Disconnected(id, reason) => {
                    let line = match self.clients.remove(&id) {
                        Some(c) => format!(
                            "client disconnected: pid {} uid {} after {} ms ({reason}); bound {:?}; cmd {:?}",
                            c.pid,
                            c.uid,
                            c.since.elapsed().as_millis(),
                            c.bound,
                            c.cmd
                        ),
                        None => format!("client disconnected: unknown ({reason})"),
                    };
                    self.journal(line);
                    // Its keyboards and pointers are dead objects now; nothing is sent on them again.
                    self.seat.keyboards.retain(|k| k.is_alive());
                    self.seat.pointers.retain(|p| p.is_alive());
                    self.seat.touches.retain(|t| t.is_alive());
                    self.seat.input_managers.retain(|m| m.is_alive());
                    if self.clients.is_empty() && self.config.exit_on_disconnect {
                        self.should_exit = true;
                    }
                }
            }
        }
    }

    /// A client was accepted: its credentials and command line are read now, while the process is certainly alive.
    /// The C backend never queues the connect event, so the listener calls this directly.
    pub fn client_connected(&mut self, id: ClientId) {
        if self.clients.contains_key(&id) {
            return;
        }
        self.stats.clients_total += 1;
        let info = match self.dh.backend_handle().get_client_credentials(id.clone()) {
            Ok(c) => ClientInfo::new(c.pid, c.uid),
            Err(_) => ClientInfo::new(0, 0),
        };
        let line = format!("client connected: pid {} uid {} cmd {:?}", info.pid, info.uid, info.cmd);
        self.clients.insert(id, info);
        self.journal(line);
    }

    /// A global was bound; kept per client for the disconnect line.
    pub fn note_bind(&mut self, client: &Client, interface: &'static str) {
        let id = client.id();
        if !self.clients.contains_key(&id) {
            self.client_connected(id.clone());
        }
        if let Some(c) = self.clients.get_mut(&id) {
            c.bound.push(interface);
        }
    }
}

impl State {
    /// Called from every dispatcher with the client: makes sure the client is known (tests insert clients without
    /// the listener).
    pub fn touch_client(&mut self, client: &Client) {
        if !self.clients.contains_key(&client.id()) {
            self.client_connected(client.id());
        }
    }
}
