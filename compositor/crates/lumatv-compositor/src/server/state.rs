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
use lumatv_wl_protocols::lumatv_subtitles_v1::server::lumatv_subtitle_manager_v1::LumatvSubtitleManagerV1;
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

// The fallback is the same panel-space band as placement::model::SUBS.
fn caption_exclusion(report: Option<Option<crate::Rect>>, eligible_app: bool) -> Option<crate::Rect> {
    match report {
        Some(rect) => rect,
        None if eligible_app => Some(crate::Rect::new(0, 905, crate::OUTPUT_WIDTH as i32, 105)),
        None => None,
    }
}

#[cfg(test)]
mod caption_exclusion_tests {
    use super::caption_exclusion;
    use crate::Rect;

    #[test]
    fn known_empty_and_bounds_override_the_unknown_app_fallback() {
        let rect = Rect::new(20, 840, 1000, 120);
        assert_eq!(caption_exclusion(Some(Some(rect)), true), Some(rect));
        assert_eq!(caption_exclusion(Some(Some(rect)), false), Some(rect));
        assert_eq!(caption_exclusion(Some(None), true), None);
        assert_eq!(caption_exclusion(None, true), Some(Rect::new(0, 905, 1920, 105)));
        assert_eq!(caption_exclusion(None, false), None);
    }
}

#[derive(Default, Debug, Clone, Copy)]
pub struct Stats {
    pub requests: u64,
    pub commits: u64,
    pub buffers_created: u64,
    pub ticks: u64,
    pub frames_drawn: u64,
    pub callbacks_sent: u64,
    /// Of those, the ones that went out before their frame landed ([`Owed::ahead`]).
    pub callbacks_ahead: u64,
    /// Of those, the ones that went out with the commit that carried them ([`ClientCallbacks::Commit`]).
    pub callbacks_at_commit: u64,
    pub releases: u64,
    pub clients_total: u64,
    pub render_us_last: u64,
    pub render_us_max: u64,
    /// What the last drawn frame owed: its rectangles and their pixels (`RenderStats::rects`, `owed_px`).
    pub rects_last: u32,
    pub owed_px_last: u64,
    pub ui_draws_last: u32,
    pub ui_layers_last: u32,
    /// Frames in which the UI engine drew something (a draw call of either stack): what the shell asks of the GPU, apart from the frames a window's own
    /// commits make (`drawn` counts both).
    pub ui_frames: u64,
    pub skipped_items: u64,
    pub gl_errors: u64,
    /// Buffers whose first-use costs the commit paid (`Output::prepare_buffer`).
    pub prepared: u64,
    /// Buffers a frame had to import itself (zero while every commit prepares its buffer).
    pub frame_imports: u64,
    /// Ticks that drew nothing because the driver was still busy with a buffer a commit had prepared.
    pub held_ticks: u64,
    /// How long the last tick took, plan to callbacks.
    pub tick_us_last: u64,
    /// The longest the loop was busy in one pass, wake-up to flush, since the start: what holds every client's callbacks and the
    /// supervisor's heartbeat (answered by the loop) back. A frame at 60 Hz is 16,667.
    pub pass_us_max: u64,
    /// The passes of the loop since the start (a wake-up to its flush) and what they took, in µs: how often the loop woke and for how long.
    pub passes: u64,
    pub pass_us_total: u64,
    /// What the ticks took since the start, in µs, and in which part of the tick ([`State::tick_at`]): the plan and the frame, the
    /// present (with the box dim), the callbacks, the releases, the rest. A TV measurement splits the loop's CPU with these.
    pub tick_us_total: u64,
    pub tick_parts_us: [u64; 5],
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
    /// The cursor standby time this client set through `wl_starfish_pointer` (ms), answered to its `get_mrcu_standby_timer`. Per
    /// client: a probe got LG's 5000 from LG's compositor, and from one value for all the 300000 another client had set (2026-10-28).
    pub mrcu_standby_ms: u32,
    /// The last moments of this client's surfaces (`closetrace`), written out when it disconnects after a destroy.
    pub trace: super::closetrace::Trace,
}

impl ClientInfo {
    fn new(pid: i32, uid: u32) -> Self {
        ClientInfo {
            pid,
            uid,
            since: Instant::now(),
            cmd: if pid > 0 { cmdline(pid) } else { String::new() },
            bound: Vec::new(),
            mrcu_standby_ms: super::starfish::MRCU_STANDBY_DEFAULT_MS,
            trace: Default::default(),
        }
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

/// When a client's frame callbacks go out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientCallbacks {
    /// A card's with the commit that carries them, before the frame is drawn; every other window's as under [`Self::Draw`].
    /// The default since round 19. Cards are the apps SAM launches, each drawing through LG's libmali; LG's own overlays and the
    /// x1 probe (an overlay) keep the draw's pacing.
    ///
    /// LG's libmali (r46p0, read from the driver) presents each swap on its own thread: `display_window_buffer` waits for the frame
    /// callback of the frame before, polling the socket a second at a time, and only then attaches and commits the next buffer on the
    /// `wl_surface` it kept. The app's swap returns two frames ahead of that thread (`eglp_gles_flush_operation`'s semaphore), so with
    /// the callback at the draw a present was waiting on the compositor at almost every moment: the commit the callback released
    /// lands a millisecond after the draw and waits a whole tick for the next. LG's SDL destroys the `wl_surface` before
    /// `eglTerminate`, which then waits for that thread (`eglp_delete_surface`); the callback goes with the surface, the thread waits
    /// out its second, and SDL's `callback_forceQuit` runs a second `SDL_Quit` under it at 1 s: PlxNative crashed in 12 of 15 closes
    /// (round 18; 10 of 10 at the flip). LG's compositor answers 5-10 ms after the commit, before the app's teardown is reached
    /// (probed 2026-10-29); at the commit the present never waits at all.
    Commit,
    /// A frame ahead of the flip ([`Owed::ahead`]), with the buffers its frame replaced; the default of rounds 16-18.
    Draw,
    /// At the flip ([`Owed::callbacks`]), the buffers given back at the draw.
    Landing,
}

impl ClientCallbacks {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "commit" => Some(Self::Commit),
            "early" => Some(Self::Draw),
            "landing" => Some(Self::Landing),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Draw => "early",
            Self::Landing => "landing",
        }
    }
}

/// What a frame on its way to the screen owes once it is there.
struct Owed {
    /// Frame callbacks, fired when the frame lands.
    callbacks: Vec<WlCallback>,
    /// Frame callbacks fired as soon as the frame is the oldest on its way (at its draw when none is ahead of it), a frame before it
    /// lands: the shell's, and every other client's while [`State::client_callbacks`] is [`ClientCallbacks::Draw`].
    ///
    /// The shell's Qt starts each frame on the callback of the one before: 2-3.5 ms of CPU, then 11-13.5 ms of GPU on the picker's
    /// slide, and a flip needs the GPU done before its refresh. Started at the landing the frame had less than a millisecond to spare,
    /// and 5 of 34 slide frames took two refreshes (round 4: the quick panel, lighter by a millisecond or two, took none).
    ///
    /// LG's apps get theirs at the same moment as the buffer they draw their next frame into, as from LG's Qt compositor, which sends
    /// frame callbacks after rendering (`afterRendering`) and gives buffers back when it renders. With the callback at the flip, 10.5 ms
    /// after the buffer, PlxNative's last swap was still waiting for it when SDL destroyed the surface: the present then ran on the
    /// freed surface (round 16, 23 of 25 closes) or, its callback destroyed with the surface, waited out libmali's 1 s, and SDL's
    /// force-quit crashed (round 17, 9 of 10; round 18, 12 of 15 at the draw). See [`ClientCallbacks::Commit`].
    ahead: Vec<WlCallback>,
    /// What the commits of the surfaces with [`Owed::ahead`] callbacks (the shell's aside) replaced: given back with those callbacks,
    /// never before the callback of the frame that replaced them.
    ahead_buffers: Vec<WlBuffer>,
    /// What the shell window's commits replaced: released when the frame lands instead of when it is drawn, since a client let run ahead
    /// takes the first buffer it is given back, and the one on the screen until this frame lands must not be drawn into.
    buffers: Vec<WlBuffer>,
}

pub struct State {
    pub dh: DisplayHandle,
    pub config: Config,
    pub refresh_policy: crate::refresh::Policy,
    pub surfaces: Surfaces,
    pub scene: Scene,
    pub output: Box<dyn Output>,
    pub seat: SeatState,
    pub foreign: ForeignState,
    pub luna: Box<dyn luna::Bridge>,
    pub luna_state: crate::luna_methods::LunaState,
    pub keys: crate::input::Keys,
    pub ime: super::ime::ImeState,
    /// The Rust shell client (`shell_rust.rs`): the kinds it answers, its channel and its supervision.
    pub shell_rust: crate::shell_rust::RustShell,
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
    /// A fatal runtime failure requires the supervisor to restore stock.
    pub fatal_error: Option<String>,
    pub last_capture: Option<PathBuf>,
    /// Captured frames are encoded and written on its thread.
    pub pngs: capture::Writer,
    /// Screenshots and screen recordings (`screencap`); idle, and costing nothing, until a command asks.
    pub screencap: crate::screencap::Screencap,
    /// The `wl_webos_shell_surface` of each window that has one.
    pub webos_shells: HashMap<crate::window::WindowId, lumatv_wl_protocols::webos_shell::server::wl_webos_shell_surface::WlWebosShellSurface>,
    pub started: Instant,
    /// When every other client's frame callbacks go out ([`ClientCallbacks`]): at the commit, a frame ahead of the flip with the
    /// buffers its frame replaced, or at the flip with the buffers given back at the draw.
    pub client_callbacks: ClientCallbacks,
    /// The Rust shell's engine socket is new (its handshake just finished): the main loop adds it to its poll set and clears this.
    pub ui_socket_new: bool,
    serial: u32,
    /// Buffers released at the end of the tick that presented the frame which stopped reading them.
    release_after_present: Vec<WlBuffer>,
    /// Buffers a client let go of that the display still reads (a direct scan-out): released once the output stops holding them
    /// ([`State::release_buffer`]), never while a frame that shows them is on the screen or on its way.
    display_held: Vec<WlBuffer>,
    /// What frames on their way to the screen owe, oldest first, paid when each lands ([`State::frames_landed`]). Frame callbacks of
    /// [`Owed::callbacks`] go out at the flip: firing at the tick paced clients to the latch, which the pacer moves by a few
    /// milliseconds whenever a slow frame enters or leaves its window; at the flip they follow the refresh. [`Owed::ahead`] go before.
    frames_in_flight: std::collections::VecDeque<(Instant, Owed)>,
    /// Per surface: when its last new buffer came and how many came in a row within 20 ms of each other, to log
    /// the gaps of a client that was keeping up (what the X1 gate counts, seen from this side).
    cadence: HashMap<SurfaceId, (Instant, u32)>,
    /// The last tick drew nothing because a driver buffer was warming (one log line per such run).
    holding: bool,
    /// The off-screen proof of route A (`--scanout-probe`, `scanout::probe`): polled every tick.
    pub probe: Option<crate::scanout::probe::Probe>,
    /// The protocol tap on the C display (`scanout::tap`), the descriptors it keeps, and the display it goes on (`None` on the Rust backend).
    /// Installed at start when direct scan-out or its proof is asked for, or by `ctl scanout direct`.
    pub tap: crate::scanout::tap::Tap,
    pub meta: crate::scanout::meta::Registry,
    pub wl_display: Option<*mut std::ffi::c_void>,
    /// The cursor plane carries the pointer (`pointer_sync`): the sprite is not drawn into the frame while this holds.
    pub pointer_carried: bool,
    /// LG's Input Manager queue (`im_queue`): the reader that answers lginput2's handshake at each reconnect of the Magic Remote and tells it
    /// when the pointer shows or hides. `None` where the compositor does not own the set's input (`--input` off, `--no-im-queue`).
    pub im: Option<crate::im_queue::ImQueue>,
    /// The hardware video windows' allocator, claims and the `videooutputd` status they read (`vwin`).
    pub vwin: crate::vwin::Vwin,
    /// The panel dim (`paneldim`): the saver's and idle Home's dim on the panel's own light, and the token of its call in flight.
    pub panel: crate::paneldim::PanelDim,
    pub panel_token: Option<u64>,
    /// `ctl paneldim want X`: a dim asked for by hand, for the TV check and the tests.
    pub panel_want_ctl: Option<f32>,
}

impl State {
    pub fn new(dh: DisplayHandle, config: Config, output: Box<dyn Output>, luna: Box<dyn luna::Bridge>) -> State {
        let keys = crate::input::Keys::new(&config);
        let config_vwin = config.vwin;
        let mut scene = Scene::default();
        scene.care = crate::oledcare::Care::new(&config.data_dir);
        let motion = crate::transition::ThemeSwitch::new(&config.data_dir);
        let panel_dir = config.data_dir.clone();
        let refresh_policy = crate::refresh::Policy::new(config.refresh_hz);
        State {
            dh,
            config,
            refresh_policy,
            surfaces: Surfaces::default(),
            scene,
            output,
            // The first turn of the wheel shows the pointer mid-screen until the remote has said where it points.
            seat: SeatState { seats: 1, pointer_position: (crate::OUTPUT_WIDTH as i32 / 2, crate::OUTPUT_HEIGHT as i32 / 2), ..SeatState::default() },
            foreign: ForeignState::default(),
            luna,
            luna_state: crate::luna_methods::LunaState::default(),
            keys,
            ime: super::ime::ImeState::default(),
            shell_rust: crate::shell_rust::RustShell::default(),
            switcher: crate::switcher::Switcher::default(),
            motion,
            stats: Stats::default(),
            check: CheckState::default(),
            clients: HashMap::new(),
            journal: Vec::new(),
            should_exit: false,
            fatal_error: None,
            last_capture: None,
            pngs: capture::Writer::default(),
            screencap: crate::screencap::Screencap::default(),
            webos_shells: HashMap::new(),
            started: Instant::now(),
            client_callbacks: ClientCallbacks::Commit,
            ui_socket_new: false,
            serial: 0,
            release_after_present: Vec::new(),
            display_held: Vec::new(),
            cadence: HashMap::new(),
            frames_in_flight: std::collections::VecDeque::new(),
            holding: false,
            probe: None,
            tap: crate::scanout::tap::Tap::off(),
            meta: crate::scanout::meta::Registry::default(),
            wl_display: None,
            pointer_carried: false,
            im: None,
            vwin: crate::vwin::Vwin::new(config_vwin),
            panel: crate::paneldim::PanelDim::new(&panel_dir),
            panel_token: None,
            panel_want_ctl: None,
        }
    }

    /// Registers every global the compositor serves. `mali_buffer_sharing` only when this compositor serves the
    /// buffers itself (`--buffers own`, or no GL at all so a client's bind at least gets an answer). The starfish
    /// globals are LG's set's (its 23 globals, probed 2026-10-28): LG's SDL binds `wl_starfish_pointer` and sends it
    /// the cursor standby timer at the bind, at every `state_changed(fullscreen)` and at every pointer enter, on a
    /// proxy that is NULL without the global (PlxNative: SIGSEGV in `SetMRCUTimer` 0.5 s after its map).
    pub fn create_globals(dh: &DisplayHandle, own_mali: bool) {
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
        dh.create_global::<State, WlStarfishPointer, ()>(1, ());
        dh.create_global::<State, WlStarfishOutput, ()>(1, ());
        dh.create_global::<State, LumatvSubtitleManagerV1, ()>(1, ());
    }

    /// One more `wl_seat` global, for an input device (`create_globals` made seat 0); returns its id, which the
    /// device's keys are sent on. The device's name is what `wl_webos_seat.info` tells clients, as LG's compositor
    /// does ("LGE RCU", "LGE M-RCU …": LG's SDL numbers the remotes by these).
    pub fn add_seat(&mut self, device: &str) -> super::seat::SeatId {
        let id = super::seat::SeatId(self.seat.seats);
        self.seat.seats += 1;
        self.seat.device_names.push(device.to_string());
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

    /// What the IM queue tells lginput2 about the pointer: whether it shows, and after how long a still one hides.
    fn im_cursor(&self) -> crate::im_queue::CursorInfo {
        crate::im_queue::CursorInfo { visible: self.seat.cursor_visible, standby_ms: super::seat::CURSOR_TIMEOUT.as_millis() as u32 }
    }

    /// The IM queue is readable: its requests are answered now, lginput2 waits 2 s for each.
    pub fn im_poll(&mut self) {
        let cursor = self.im_cursor();
        let Some(im) = &mut self.im else { return };
        let mut lines = Vec::new();
        im.poll(cursor, &mut |l| lines.push(l));
        for l in lines {
            self.journal(l);
        }
    }

    /// The pointer showed or hid: told to lginput2 over the IM queue once it has registered for it.
    pub fn im_cursor_visible(&mut self, visible: bool) {
        let Some(im) = &mut self.im else { return };
        let mut lines = Vec::new();
        im.cursor_visibility(visible, &mut |l| lines.push(l));
        for l in lines {
            self.journal(l);
        }
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
        let client = self.seat.surfaces.get(&id).and_then(|s| s.client()).map(|c| c.id());
        let buffer = self.surfaces.get(id).and_then(|s| s.current.buffer.as_ref()).map_or(0, |b| b.id().protocol_id());
        self.trace_client(client, "commit", buffer, false);
        let mut mapping_changed = false;
        for (sid, eff) in effects {
            if eff.new_buffer
                && let Some(s) = self.surfaces.get(sid)
                && s.current.size.is_none()
            {
                log::warn!("surface {} committed a buffer of unknown size", sid.0);
            }
            if eff.mapped_changed.is_some() || eff.geometry_changed {
                self.scene.invalidate_video_holes_for_surface(sid, &self.surfaces);
                mapping_changed = true;
            }
            if let Some(mapped) = eff.mapped_changed {
                let card = self.scene.window_of(sid).and_then(|id| self.scene.window(id)).is_some_and(|w| w.kind.is_card() && !w.in_group && w.state != crate::window::ShellState::Minimized);
                let was_front = !mapped && card && self.scene.windows().filter(|w| w.kind.is_card() && !w.in_group && w.front > 0 && w.state != crate::window::ShellState::Minimized && (w.map_serial == 0 || w.surface == sid || self.surfaces.get(w.surface).is_some_and(|s| s.is_mapped()))).max_by_key(|w| w.front).is_some_and(|w| w.surface == sid);
                if card && (mapped || was_front) { self.scene.invalidate_planned_video_holes(); }
                self.scene.set_mapped(sid, mapped);
                if let Some(wid) = self.scene.window_of(sid) {
                    let w = self.scene.window(wid).unwrap();
                    let (is_card, is_saver) = (w.kind.is_card(), w.kind == crate::window::WindowType::Screensaver);
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
                    } else if is_saver {
                        super::webos_shell::sync_exposed(self);
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
            // A card's callbacks go out with the commit that carried them (`ClientCallbacks::Commit`); the other windows' and
            // the shell's wait for their frame's draw, as do every client's under the other two settings.
            if self.client_callbacks == ClientCallbacks::Commit && self.is_card_surface(sid) {
                let callbacks = self.surfaces.take_frame_callbacks_of(sid);
                if !callbacks.is_empty() {
                    self.stats.callbacks_at_commit += callbacks.len() as u64;
                    self.fire_callbacks(callbacks, crate::output::monotonic_ms(), "callbacks-at-commit");
                }
            }
        }
        if mapping_changed {
            self.luna_notify();
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
        // None of the surface's frame callbacks is answered after it is gone, wherever it waits (pending, committed, or with a frame
        // on its way): each is destroyed, as LG's compositor does. With all of them answered after the destroy, PlxNative crashed in
        // libwayland-client on a freed proxy (wl_proxy_create_wrapper, and a request marshalled with no signature) in 23 of 25 closes
        // on 2026-10-28; with those of frames already on their way answered, in 4 of 13 (round 15); never under LG's compositor.
        let mut gone = Vec::new();
        if let Some(s) = self.surfaces.remove(id) {
            for b in s.current.buffer.into_iter().chain(s.current.retired) {
                self.release_after_present.push(b);
            }
            gone.extend(s.pending.frame_callbacks);
            gone.extend(s.cached.into_iter().flat_map(|c| c.frame_callbacks));
            gone.extend(s.current.frame_callbacks);
        }
        for (_, owed) in self.frames_in_flight.iter_mut() {
            for list in [&mut owed.callbacks, &mut owed.ahead] {
                if list.iter().any(|cb| cb.data::<SurfaceId>() == Some(&id)) {
                    let (of_it, kept): (Vec<_>, Vec<_>) = std::mem::take(list).into_iter().partition(|cb| cb.data::<SurfaceId>() == Some(&id));
                    *list = kept;
                    gone.extend(of_it);
                }
            }
        }
        self.destroy_callbacks(gone);
        let window = self.scene.window_of(id).and_then(|wid| self.scene.window(wid)).map(|w| w.kind.is_card());
        self.scene.remove_surface(id);
        super::ime::surface_gone(self, id);
        self.cadence.remove(&id);
        self.foreign.surface_destroyed(id);
        match window {
            Some(was_card) => super::webos_shell::window_gone(self, was_card),
            None => {
                self.shell_rust_sync_topics();
                super::seat::refocus(self);
            }
        }
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
        let prev_tick_us = self.stats.tick_us_last;
        self.probe_poll();
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
        // The pointer: on the cursor plane when it carries it (no sprite in the frame, no damage for a move), else a sprite in the frame.
        self.pointer_sync();
        self.vwin_sync();
        self.scene.cursor = if self.seat.cursor_visible && !self.pointer_carried {
            Some(crate::render::cursor::rect_at(self.seat.pointer_position.0, self.seat.pointer_position.1))
        } else {
            None
        };
        let home_before_plan = self.shell_home_drawn();
        self.switcher_frame(t0);
        self.scene.transitions.suspended = self.switcher_active();
        // The screen is off (standby): nothing is mid-move when it comes back.
        if !self.luna_state.power.rendering && self.scene.transitions.active() {
            self.scene.transitions.clear();
        }
        self.panel.settings = self.scene.care.settings.paneldim;
        self.panel.set_bridge(self.luna.is_connected());
        // LG's own saver over an app is a saver of its own, with its own burn-in care: the app under it is not dimmed, shifted or sampled
        let lg_saver = self.lg_saver_up();
        let State { scene, surfaces, output, stats, shell_rust, screencap, panel, .. } = self;
        scene.care.refresh(now_ns);
        // while the panel is the one dimming Home, Home is drawn at full brightness (and may go to the plane whole)
        scene.care.panel_carry = panel.carries() || panel.dimmed();
        scene.care.saver = lg_saver || shell_rust.ui_state.get("saver").and_then(serde_json::Value::as_bool) == Some(true);
        scene.boxdim.capturing = screencap.armed();
        // A box dim sample the GPU has finished (`boxdim`): judged before this plan, so what it saw changed is in this frame.
        if scene.boxdim.in_flight() {
            let began = Instant::now();
            match output.sample_take(&mut scene.boxdim.incoming) {
                Ok(true) => scene.boxdim.take_sample(now_ns, began.elapsed().as_micros() as u32),
                Ok(false) => {}
                Err(e) => {
                    scene.boxdim.sample_failed(now_ns);
                    log::warn!("boxdim: {e}");
                }
            }
        }
        scene.plan_at(surfaces, now_ns);
        let eligible_app = scene.foreground(surfaces).is_some_and(|front| {
            front.app_id() != crate::scene::HOME_APP
                && scene.last_plan().items.iter().any(|item| item.window == front.id && !item.ghost)
        });
        let report = scene.subtitle_bounds(surfaces);
        let exclusion = caption_exclusion(report.map(|(_, _, rect)| rect), eligible_app);
        output.ui_set_exclusion(exclusion);
        let root = report.and_then(|(_, root, _)| match scene.last_plan().items.as_slice() {
            [item] if item.surface == root && !item.ghost && item.pose.is_none() && !item.xform
                && item.drift == (0, 0)
                && item.rect == crate::Rect::new(0, 0, crate::OUTPUT_WIDTH as i32, crate::OUTPUT_HEIGHT as i32) => Some(root),
            _ => None,
        });
        output.ui_set_standin_root(root);
        let plan = scene.last_plan();
        let mut drew = false;
        let ui_changed = output.ui_needs_frame().unwrap_or_else(|e| {
            log::error!("system UI: {e}");
            false
        });
        if !plan.damage.is_empty() || ui_changed {
            match output.render(plan, surfaces) {
                Ok(st) => {
                    drew = true;
                    scene.transitions.drawn(st.posed_waiting == 0);
                    stats.render_us_last = st.render_us;
                    stats.rects_last = st.rects;
                    stats.owed_px_last = st.owed_px;
                    stats.ui_draws_last = st.ui_draws;
                    stats.ui_layers_last = st.ui_layers;
                    stats.ui_frames += u64::from(st.ui_draws > 0);
                    stats.render_us_max = stats.render_us_max.max(st.render_us);
                    stats.skipped_items += st.skipped as u64;
                    stats.gl_errors = st.gl_errors;
                    stats.frame_imports += st.imports as u64;
                }
                Err(e) => log::error!("render: {e}"),
            }
        }
        let t_rendered = Instant::now();
        for line in self.output.take_journal() {
            self.journal(line);
        }
        // While a transition moves the frees of destroyed buffers wait (the flip of this frame lands after this).
        self.output.defer_frees(self.scene.transitions.animating(now_ns));
        if drew && self.scene.care_frame() {
            self.output.quiet_frame();
        }
        for line in self.scene.care.take_events().into_iter().chain(self.scene.boxdim.take_journal()) {
            self.journal(line);
        }
        self.panel_dim_tick(now_ns);
        let presented = match self.output.present() {
            Ok(p) => p,
            Err(e) => {
                log::error!("present: {e}");
                Presented::default()
            }
        };
        // The box dim (`boxdim`): a sample of the app's pictures begun after the frame (the GPU's work, read on a later tick), or a piece of
        // what sampling needs made on a tick with nothing else to do. Never while a driver buffer warms: a draw from it would wait.
        let State { scene, surfaces, output, .. } = self;
        if !output.warming() {
            if scene.boxdim.wants_sample(now_ns, drew) {
                let began = Instant::now();
                match output.sample_begin(scene.last_plan(), surfaces) {
                    Ok(true) => scene.boxdim.sample_started(now_ns, began.elapsed().as_micros() as u32),
                    Ok(false) => scene.boxdim.sample_skipped(now_ns),
                    Err(e) => {
                        if scene.boxdim.sample_unavailable(now_ns) && e.kind() != std::io::ErrorKind::Unsupported {
                            log::warn!("boxdim: no sample ({e}): damage is taken for change");
                        }
                    }
                }
            } else if scene.boxdim.wants_warm(now_ns, drew) {
                let began = Instant::now();
                let done = output.boxdim_warm().unwrap_or(true);
                scene.boxdim.warmed(began.elapsed().as_micros() as u32, done);
            }
        }
        let t_presented = Instant::now();
        self.land_flips();
        if drew {
            self.stats.frames_drawn += 1;
            self.after_drawn_frame();
        }
        let State { surfaces, client_callbacks, .. } = &mut *self;
        let early = *client_callbacks != ClientCallbacks::Landing;
        let (callbacks, ahead) = surfaces.take_frame_callbacks(|_| early);
        let [retired, ahead_buffers, buffers] = surfaces.take_retired(|_| usize::from(early));
        self.route_callbacks(Owed { callbacks, ahead, ahead_buffers, buffers }, presented);
        let t_callbacks = Instant::now();
        // What clients let go of: released now, or kept while the display still reads it.
        let mut gone = std::mem::take(&mut self.release_after_present);
        for b in gone.drain(..).chain(retired) {
            self.release_buffer(b);
        }
        self.release_after_present = gone;
        self.release_displayed();
        self.surfaces.clear_damage();
        let t_released = Instant::now();
        if home_before_plan != self.shell_home_drawn() {
            self.luna_notify();
        } else {
            self.shell_rust_sync_video();
        }
        self.shell_rust_sync_subtitle_bounds();
        self.screencap_tick(drew);
        if let Some(n) = self.config.frames
            && self.stats.frames_drawn >= n
        {
            self.should_exit = true;
        }
        let t_end = Instant::now();
        self.stats.tick_us_last = (t_end - t0).as_micros() as u64;
        self.stats.tick_us_total += self.stats.tick_us_last;
        for (sum, part) in self.stats.tick_parts_us.iter_mut().zip([t_rendered - t0, t_presented - t_rendered, t_callbacks - t_presented, t_released - t_callbacks, t_end - t_released]) {
            *sum += part.as_micros() as u64;
        }
        // A frame of a transition that came late says in what part and what the loop had been doing: a long tick before it is the loop's own
        // work (a free, an import, a draw); short ones around a long gap are the loop waiting or woken late (the display, the driver, another process).
        if let Some(l) = self.scene.transitions.take_late() {
            self.journal(format!(
                "transitions: frame {} ms after the last in a {} (tick before {:.1} ms, this {:.1} ms, last render {:.1} ms, held ticks {}, buffers prepared {}, imported in a frame {})",
                l.gap_ms,
                l.phase,
                prev_tick_us as f64 / 1000.0,
                self.stats.tick_us_last as f64 / 1000.0,
                self.stats.render_us_last as f64 / 1000.0,
                self.stats.held_ticks,
                self.stats.prepared,
                self.stats.frame_imports
            ));
        }
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
    /// once when none is. The oldest frame in flight has its early callbacks fired at once ([`Owed::ahead`]).
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
            last.ahead_buffers.extend(owed.ahead_buffers);
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
        if n > 0 {
            self.release_displayed();
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

    /// Whether `id` is a card window's surface (`WindowType::is_card`).
    fn is_card_surface(&self, id: SurfaceId) -> bool {
        self.scene.window_of(id).and_then(|w| self.scene.window(w)).is_some_and(|w| w.kind.is_card())
    }

    /// Notes `what` in the close trace of `client` (a closing client's last moments are logged when it disconnects, `closetrace`).
    pub fn trace_client(&mut self, client: Option<ClientId>, what: &'static str, n: u32, destroy: bool) {
        if let Some(c) = client.and_then(|id| self.clients.get_mut(&id)) {
            c.trace.note(what, n, destroy);
        }
    }

    fn fire_callbacks(&mut self, callbacks: Vec<WlCallback>, at_ms: u32, why: &'static str) {
        for cb in callbacks {
            if cb.is_alive() {
                cb.done(at_ms);
                self.stats.callbacks_sent += 1;
                self.trace_client(cb.client().map(|c| c.id()), why, 1, false);
            }
        }
    }

    /// Destroys frame callbacks that will never be answered (their surface is gone), as `wl_resource_destroy` does: the client gets
    /// `delete_id` for each and never `done`.
    fn destroy_callbacks(&mut self, callbacks: Vec<WlCallback>) {
        for cb in callbacks {
            if cb.is_alive() {
                let client = cb.client().map(|c| c.id());
                let _ = self.dh.backend_handle().destroy_object::<State>(&cb.id());
                self.trace_client(client, "callback-destroyed", 1, false);
            }
        }
    }

    /// A frame is on the screen, or none is on its way: its callbacks are due and what it stopped reading is free.
    fn pay(&mut self, owed: Owed, at_ms: u32) {
        self.fire_callbacks(owed.ahead, at_ms, "callbacks-ahead");
        self.release(owed.ahead_buffers);
        self.fire_callbacks(owed.callbacks, at_ms, "callbacks-landed");
        self.release(owed.buffers);
    }

    /// The oldest frame in flight has no frame ahead of it: its early callbacks go out now, with the buffers its commits replaced,
    /// so the client's next frame is made while this one waits for its refresh.
    fn fire_ahead(&mut self, at_ms: u32) {
        let Some((_, oldest)) = self.frames_in_flight.front_mut() else { return };
        let ahead = std::mem::take(&mut oldest.ahead);
        let buffers = std::mem::take(&mut oldest.ahead_buffers);
        self.stats.callbacks_ahead += ahead.len() as u64;
        self.fire_callbacks(ahead, at_ms, "callbacks-ahead");
        self.release(buffers);
    }

    fn release(&mut self, buffers: Vec<WlBuffer>) {
        for b in buffers {
            self.release_buffer(b);
        }
    }

    /// Tells a client it may draw into `b` again, unless the display reads it (a direct scan-out): then it waits in `display_held` for the
    /// frame that replaces it to land.
    fn release_buffer(&mut self, b: WlBuffer) {
        if !b.is_alive() {
            return;
        }
        if self.output.holds(&b) {
            if !self.display_held.iter().any(|h| h.id() == b.id()) {
                self.display_held.push(b);
            }
        } else {
            let client = b.client().map(|c| c.id());
            self.trace_client(client, "release", b.id().protocol_id(), false);
            b.release();
            self.stats.releases += 1;
        }
    }

    /// Releases what waited for the display, now that it no longer reads it.
    fn release_displayed(&mut self) {
        let mut i = 0;
        while i < self.display_held.len() {
            if !self.display_held[i].is_alive() {
                self.display_held.swap_remove(i);
            } else if self.output.holds(&self.display_held[i]) {
                i += 1;
            } else {
                let b = self.display_held.swap_remove(i);
                let client = b.client().map(|c| c.id());
                self.trace_client(client, "release", b.id().protocol_id(), false);
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

    /// What the compositor holds that clients made, by kind (`ctl status` `held`): a client that goes, cleanly or by dying mid-teardown,
    /// takes its share with it, so these counts come back to where they were (the crash-loop test).
    pub fn held(&self) -> serde_json::Value {
        let owed = |f: fn(&Owed) -> usize| self.frames_in_flight.iter().map(|(_, o)| f(o)).sum::<usize>();
        serde_json::json!({
            "surfaces": self.surfaces.len(),
            "windows": self.scene.windows().count(),
            "webosShells": self.webos_shells.len(),
            "cadence": self.cadence.len(),
            "framesInFlight": self.frames_in_flight.len(),
            "owedCallbacks": owed(|o| o.callbacks.len() + o.ahead.len()),
            "owedBuffers": owed(|o| o.buffers.len() + o.ahead_buffers.len()),
            "releaseAfterPresent": self.release_after_present.len(),
            "displayHeld": self.display_held.len(),
            "seat": {
                "keyboards": self.seat.keyboards.len(), "pointers": self.seat.pointers.len(), "touches": self.seat.touches.len(),
                "inputManagers": self.seat.input_managers.len(), "surfaces": self.seat.surfaces.len(), "pressed": self.seat.pressed.len(),
            },
            "exported": self.foreign.exported.len(),
            "imported": self.foreign.imported.len(),
            "groups": self.scene.groups.len(),
            "holes": self.scene.holes.len(),
            "lunaSubscriptions": self.luna_state.subscriptions.len(),
        })
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

    /// The off-screen proof: records and imports the descriptors the tap has kept since the last look, and journals each. A tick and the end of a run
    /// do it; without a probe it is nothing.
    pub fn probe_poll(&mut self) {
        let lines = match &mut self.probe {
            Some(p) => p.poll(&self.meta),
            None => return,
        };
        for line in lines {
            self.journal(line);
        }
    }

    /// Direct scan-out for the control socket: the output's counters and state (the KMS output's), the protocol tap's, and the off-screen
    /// proof's.
    pub fn scanout_report(&self) -> serde_json::Value {
        let mut v = self.output.scanout_report().unwrap_or_else(|| {
            let m = self.meta.stats();
            serde_json::json!({
                "mode": self.config.scanout.mode.name(),
                "output": self.output.name(),
                "descriptors": {"retained": m.retained, "created": m.created, "destroyed": m.destroyed, "withClient": m.with_client, "undecoded": m.undecoded, "allocDevices": self.meta.alloc_devices()},
            })
        });
        v["tap"] = self.tap.status().report();
        if let Some(p) = &self.probe {
            v["probe"] = p.report();
        }
        v
    }

    /// `ctl scanout off|direct`: the output's switch. Direct needs the descriptors of the driver's buffers, so the tap goes on the display now if it is
    /// not on yet; the buffers a client made before that have no descriptor and are composed (`no-descriptor`) until the client makes new ones.
    pub fn set_scanout(&mut self, mode: crate::scanout::Mode) -> std::io::Result<()> {
        if mode == crate::scanout::Mode::Direct && self.tap.status() == crate::scanout::tap::Status::Off {
            self.tap = crate::scanout::tap::Tap::install(self.wl_display, &self.meta);
            self.journal(format!("scanout: protocol tap {}", self.tap.status().report()));
        }
        self.output.set_scanout(mode)?;
        self.config.scanout.mode = mode;
        Ok(())
    }

    /// `ctl scanout offset on|off`: whether a direct frame may carry the OLED care's orbit as the plane's position (`scanout::Policy::offset`).
    pub fn set_scanout_offset(&mut self, on: bool) -> std::io::Result<()> {
        self.output.set_scanout_offset(on)?;
        self.config.scanout.offset = on;
        Ok(())
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
                    let (line, trace) = match self.clients.remove(&id) {
                        Some(c) => (
                            format!(
                                "client disconnected: pid {} uid {} after {} ms ({reason}); bound {:?}; cmd {:?}",
                                c.pid,
                                c.uid,
                                c.since.elapsed().as_millis(),
                                c.bound,
                                c.cmd
                            ),
                            c.trace.report(c.pid),
                        ),
                        None => (format!("client disconnected: unknown ({reason})"), None),
                    };
                    self.journal(line);
                    if let Some(trace) = trace {
                        log::info!("{trace}");
                    }
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
