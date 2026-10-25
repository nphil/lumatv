//! The compositor's state: everything the protocol handlers mutate, the globals, and the frame tick.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use wayland_backend::server::{ClientData, ClientId, DisconnectReason};
use wayland_server::protocol::wl_buffer::WlBuffer;
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
    pub releases: u64,
    pub clients_total: u64,
    pub render_us_last: u64,
    pub render_us_max: u64,
    pub skipped_items: u64,
}

#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub pid: i32,
    pub uid: u32,
    pub since: Instant,
}

/// The X2 self-check: counts drawn frames whose probe pixels match the pattern.
#[derive(Default, Debug, Clone, Copy)]
pub struct CheckState {
    pub next_frame: u32,
    pub ok: u64,
    pub bad: u64,
    pub unread: u64,
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
    pub stats: Stats,
    pub check: CheckState,
    pub clients: HashMap<ClientId, ClientInfo>,
    /// Recent protocol-level events worth showing on the control socket (bounded).
    pub journal: Vec<String>,
    pub should_exit: bool,
    pub last_capture: Option<PathBuf>,
    /// The `wl_webos_shell_surface` of each window that has one.
    pub webos_shells: HashMap<crate::window::WindowId, lumatv_wl_protocols::webos_shell::server::wl_webos_shell_surface::WlWebosShellSurface>,
    pub started: Instant,
    serial: u32,
    /// Buffers released at the end of the tick that presented the frame which stopped reading them.
    release_after_present: Vec<WlBuffer>,
}

impl State {
    pub fn new(dh: DisplayHandle, config: Config, output: Box<dyn Output>, luna: Box<dyn luna::Bridge>) -> State {
        State {
            dh,
            config,
            surfaces: Surfaces::default(),
            scene: Scene::default(),
            output,
            seat: SeatState::default(),
            foreign: ForeignState::default(),
            luna,
            stats: Stats::default(),
            check: CheckState::default(),
            clients: HashMap::new(),
            journal: Vec::new(),
            should_exit: false,
            last_capture: None,
            webos_shells: HashMap::new(),
            started: Instant::now(),
            serial: 0,
            release_after_present: Vec::new(),
        }
    }

    /// Registers every global the compositor serves. `mali_buffer_sharing` only when this compositor serves the
    /// buffers itself (`--buffers own`, or no GL at all so a client's bind at least gets an answer).
    pub fn create_globals(dh: &DisplayHandle, config: &Config, own_mali: bool) {
        dh.create_global::<State, WlCompositor, ()>(4, ());
        dh.create_global::<State, WlSubcompositor, ()>(1, ());
        dh.create_global::<State, WlShm, ()>(1, ());
        dh.create_global::<State, WlShell, ()>(1, ());
        dh.create_global::<State, WlSeat, ()>(4, ());
        dh.create_global::<State, WlOutput, ()>(2, ());
        dh.create_global::<State, WlDataDeviceManager, ()>(1, ());
        dh.create_global::<State, WlWebosShell, ()>(2, ());
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
                        super::seat::refocus(self);
                    }
                }
            } else if eff.new_buffer {
                self.scene.layout_changed |= self.surfaces.get(sid).map(|s| s.current.damage.is_none()).unwrap_or(false);
            }
        }
    }

    pub fn surface_destroyed(&mut self, id: SurfaceId) {
        if let Some(s) = self.surfaces.remove(id) {
            for b in s.current.buffer.into_iter().chain(s.current.retired) {
                self.release_after_present.push(b);
            }
            for cb in s.current.frame_callbacks {
                // A callback on a destroyed surface is never fired; dropping the object destroys it client-side.
                drop(cb);
            }
        }
        self.scene.remove_surface(id);
        self.foreign.surface_destroyed(id);
        super::seat::refocus(self);
    }

    /// One frame: plan, draw what changed, present, pay out frame callbacks and buffer releases.
    pub fn tick(&mut self) -> Presented {
        self.stats.ticks += 1;
        let plan = self.scene.plan(&self.surfaces);
        let mut drew = false;
        if !plan.damage.is_empty() {
            match self.output.render(&plan, &self.surfaces) {
                Ok(st) => {
                    drew = st.drawn > 0 || !plan.items.is_empty() || plan.damage.w > 0;
                    self.stats.render_us_last = st.render_us;
                    self.stats.render_us_max = self.stats.render_us_max.max(st.render_us);
                    self.stats.skipped_items += st.skipped as u64;
                }
                Err(e) => log::error!("render: {e}"),
            }
        }
        let presented = match self.output.present() {
            Ok(p) => p,
            Err(e) => {
                log::error!("present: {e}");
                Presented::default()
            }
        };
        if drew {
            self.stats.frames_drawn += 1;
            self.after_drawn_frame(&plan);
        }
        for cb in self.surfaces.take_frame_callbacks() {
            cb.done(presented.at_ms);
            self.stats.callbacks_sent += 1;
        }
        let retired = self.surfaces.take_retired();
        for b in self.release_after_present.drain(..).chain(retired) {
            if b.is_alive() {
                b.release();
                self.stats.releases += 1;
            }
        }
        self.surfaces.clear_damage();
        if let Some(n) = self.config.frames
            && self.stats.frames_drawn >= n
        {
            self.should_exit = true;
        }
        presented
    }

    fn after_drawn_frame(&mut self, plan: &crate::scene::Plan) {
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
            let probe = plan.items.first().map(|i| (i.rect.x.max(0) as u32, i.rect.y.max(0) as u32, i.rect.w as u32, i.rect.h as u32)).unwrap_or((0, 0, image.width, image.height));
            let ok = capture::matches(&image, probe, expected);
            let px = image.pixel(probe.0 + probe.2 / 2, probe.1 + probe.3 / 2);
            self.journal(format!(
                "check x2 frame {k}: centre {:02x}{:02x}{:02x}{:02x} expected {:02x}{:02x}{:02x}{:02x} {}",
                px[0], px[1], px[2], px[3], expected[0], expected[1], expected[2], expected[3], if ok { "ok" } else { "BAD" }
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
            match capture::write_png(&path, &image) {
                Ok(()) => self.last_capture = Some(path),
                Err(e) => log::warn!("capture {}: {e}", path.display()),
            }
        }
    }

    /// Writes the current output to `path` on demand (control socket).
    pub fn capture_to(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        let image = self.output.capture()?;
        capture::write_png(path, &image)
    }

    /// One line per field, for the control socket and the exit summary.
    pub fn summary(&self) -> String {
        let s = &self.stats;
        format!(
            "uptime {:.1}s clients {} (total {}) surfaces {} windows {} requests {} commits {} buffers {} ticks {} drawn {} callbacks {} releases {} skipped {} render last {} us max {} us check ok {} bad {} unread {}",
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
            s.render_us_last,
            s.render_us_max,
            self.check.ok,
            self.check.bad,
            self.check.unread
        )
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
                ClientEvent::Connected(id) => {
                    self.stats.clients_total += 1;
                    self.clients.insert(id, ClientInfo { pid: 0, uid: 0, since: Instant::now() });
                    self.journal("client connected".to_string());
                }
                ClientEvent::Disconnected(id, reason) => {
                    let pid = self.clients.remove(&id).map(|c| c.pid).unwrap_or(0);
                    self.journal(format!("client disconnected: pid {pid} ({reason})"));
                    if self.clients.is_empty() && self.config.exit_on_disconnect {
                        self.should_exit = true;
                    }
                }
            }
        }
    }
}

impl State {
    /// Called from every dispatcher with the client: fills in credentials the connect event could not.
    pub fn touch_client(&mut self, client: &Client) {
        let id = client.id();
        let known = self.clients.get(&id).map(|c| c.pid != 0).unwrap_or(false);
        if known {
            return;
        }
        if let Ok(c) = client.get_credentials(&self.dh) {
            let entry = self.clients.entry(id).or_insert(ClientInfo { pid: 0, uid: 0, since: Instant::now() });
            if entry.pid == 0 {
                entry.pid = c.pid;
                entry.uid = c.uid;
                let line = format!("client pid {} uid {}", c.pid, c.uid);
                self.journal(line);
            }
        }
    }
}
