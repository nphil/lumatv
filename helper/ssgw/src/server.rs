//! The event loop: connections handed over by `ssgw-accept`, the control socket, the TV's power state and mobile-access
//! setting, and the sessions. One thread around `poll`.

use crate::adapter::{Icons, Launches};
use crate::iface::Catalog;
use crate::luna::{self, Event, Luna, Owner};
use crate::net::{self, Conn};
use crate::pins;
use crate::session::{Session, Which};
use crate::store::Store;
use crate::util::truthy;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Exit status for an environment the gateway cannot run in (no key or certificate, LG's files changed): the supervisor
/// stays on LG's gateway instead of retrying.
pub const EXIT_UNUSABLE: i32 = 78;
/// Connections served at once; more are left to LG's gateway.
const MAX_SESSIONS: usize = 16;
const HANDOFF_WAIT: Duration = Duration::from_secs(2);
const CONTROL_WAIT: Duration = Duration::from_secs(1);
/// LG retries an unreadable mobile-access setting once a minute.
const WATCH_RETRY: Duration = Duration::from_secs(60);
const STOP_WAIT: Duration = Duration::from_secs(2);
const RECENT_KEPT: usize = 20;
const RECENT_URI_MAX: usize = 256;
const RESOURCES_KEPT: usize = 1024;
pub(crate) const GATEWAY_DIR: &str = "usr/palm/services/com.webos.service.secondscreen.gateway";
const POWER: &str = "luna://com.webos.service.tvpower/power/getPowerState";
const SETTINGS: &str = "luna://com.webos.settingsservice/getSystemSettings";
const SSL_KEY: &str = "luna://com.webos.service.sm/crypto/getSSGSSL";
const OP_POWER: u32 = 1;
const OP_SETTINGS: u32 = 2;

pub struct Config {
    /// Where the client store lives.
    pub state_dir: PathBuf,
    /// The accept and control sockets and toast icons.
    pub run_dir: PathBuf,
    /// The root LG's files are read under (`/` on the set).
    pub lg_root: PathBuf,
    /// LG's gateway on its plain port.
    pub lg_http: String,
    pub luna_send: PathBuf,
    /// Serve TLS with LG's certificate and key; without it, connections are served plain (tests).
    pub tls: bool,
    /// Refuse to start unless LG's gateway files match the hashes this gateway was checked against.
    pub check_pins: bool,
    /// A `SOCK_SEQPACKET` end that opens the root-only input sockets on the gateway's behalf.
    pub broker: Option<OwnedFd>,
    pub version: String,
}

/// What sessions share.
pub(crate) struct Ctx {
    pub lg_http: String,
    first_use: PathBuf,
    pub catalog: Catalog,
    pub store: Store,
    pub luna: Luna,
    pub resources: Resources,
    pub hello: Value,
    recent: VecDeque<String>,
    pub standby: bool,
    pub launches: Launches,
    pub icons: Icons,
    pub broker: Option<Broker>,
}

impl Ctx {
    /// Remembers a request's URI for the control socket's `recent` (never payloads).
    pub fn note(&mut self, uri: &str) {
        let mut end = uri.len().min(RECENT_URI_MAX);
        while !uri.is_char_boundary(end) {
            end -= 1;
        }
        if self.recent.len() == RECENT_KEPT {
            self.recent.pop_front();
        }
        self.recent.push_back(uri[..end].to_owned());
    }

    /// LG lets clients pair without a prompt's first-use checks once the TV's first-use setup has run.
    pub fn first_use_done(&self) -> bool {
        self.first_use.exists()
    }
}

/// Files and sockets handed out as `/resources/…` URLs, by escaped path.
#[derive(Default)]
pub(crate) struct Resources {
    map: HashMap<String, (String, bool)>,
    order: VecDeque<String>,
}

impl Resources {
    pub fn add(&mut self, path: &str, file: &str, socket: bool) {
        if self.map.insert(path.to_owned(), (file.to_owned(), socket)).is_some() {
            self.order.retain(|p| p != path);
        }
        self.order.push_back(path.to_owned());
        while self.order.len() > RESOURCES_KEPT {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }

    pub fn contains(&self, path: &str) -> bool {
        self.map.contains_key(path)
    }

    pub fn media(&self, path: &str) -> Option<&str> {
        self.map.get(path).filter(|(_, socket)| !socket).map(|(f, _)| f.as_str())
    }

    /// A socket resource is good for one connection.
    pub fn take_socket(&mut self, path: &str) -> Option<String> {
        if !self.map.get(path).is_some_and(|(_, socket)| *socket) {
            return None;
        }
        self.order.retain(|p| p != path);
        self.map.remove(path).map(|(f, _)| f)
    }
}

/// The helper end that opens the input sockets: one datagram per request, an absolute path; one answer each, in order:
/// `K` with the descriptor, or `E<errno>`.
pub(crate) struct Broker {
    fd: OwnedFd,
    waiting: VecDeque<u32>,
    out: VecDeque<Vec<u8>>,
    gone: bool,
}

impl Broker {
    fn new(fd: OwnedFd) -> Broker {
        net::set_cloexec(fd.as_raw_fd());
        Broker { fd, waiting: VecDeque::new(), out: VecDeque::new(), gone: false }
    }

    /// Asks for `path` for session `sess`; `false` once the broker has gone.
    pub fn request(&mut self, sess: u32, path: &str) -> bool {
        if self.gone {
            return false;
        }
        self.out.push_back(path.as_bytes().to_vec());
        self.waiting.push_back(sess);
        self.flush();
        !self.gone
    }

    fn flush(&mut self) {
        while let Some(d) = self.out.front() {
            // SAFETY: sends from a live buffer on a descriptor the broker owns.
            let n = unsafe { libc::send(self.fd.as_raw_fd(), d.as_ptr().cast(), d.len(), libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) {
                    return;
                }
                self.lost(&e.to_string());
                return;
            }
            self.out.pop_front();
        }
    }

    fn lost(&mut self, why: &str) {
        if !self.gone {
            crate::error!("the input-socket broker went away ({why}); pointer sockets now fail");
        }
        self.gone = true;
        self.out.clear();
    }

    /// The answers that have arrived, by session.
    fn read(&mut self) -> Vec<(u32, Result<OwnedFd, i32>)> {
        let mut got = Vec::new();
        while !self.gone {
            match net::recv_fd(self.fd.as_raw_fd(), 16) {
                Ok(None) => break,
                Ok(Some(r)) if r.data.is_empty() => self.lost("closed"),
                Ok(Some(r)) => {
                    let Some(sess) = self.waiting.pop_front() else { continue };
                    let answer = match (r.data[0], r.fd) {
                        (b'K', Some(fd)) => Ok(fd),
                        (b'E', _) => Err(std::str::from_utf8(&r.data[1..]).ok().and_then(|s| s.parse().ok()).unwrap_or(libc::EIO)),
                        _ => Err(libc::EPROTO),
                    };
                    got.push((sess, answer));
                }
                Err(e) => self.lost(&e.to_string()),
            }
        }
        if self.gone {
            got.extend(self.waiting.drain(..).map(|s| (s, Err(libc::EPIPE))));
        }
        got
    }
}

struct Handoff {
    stream: UnixStream,
    deadline: Instant,
}

struct Control {
    stream: UnixStream,
    buf: Vec<u8>,
    deadline: Instant,
}

#[derive(Clone, Copy)]
enum Slot {
    Stop,
    Accept,
    Control,
    Handoff(usize),
    Ctl(usize),
    Sess(u32, Which),
    Luna(u32),
    Broker,
}

struct Server {
    ctx: Ctx,
    sessions: HashMap<u32, Session>,
    next_id: u32,
    accept: UnixListener,
    control: UnixListener,
    paths: [PathBuf; 2],
    handoffs: Vec<Handoff>,
    controls: Vec<Control>,
    tls: Option<Arc<rustls::ServerConfig>>,
    started: Instant,
    version: String,
    access: bool,
    retry_power: Option<Instant>,
    retry_settings: Option<Instant>,
    stop_by: Option<Instant>,
}

/// Runs the gateway until `stop` becomes readable; returns the process exit status.
pub fn run(cfg: Config, stop: OwnedFd) -> i32 {
    run_notify(cfg, stop, || {})
}

/// [`run`], calling `ready` once the sockets listen.
pub(crate) fn run_notify(cfg: Config, stop: OwnedFd, ready: impl FnOnce()) -> i32 {
    let mut server = match Server::start(cfg) {
        Ok(s) => s,
        Err(code) => return code,
    };
    ready();
    server.serve(stop.as_raw_fd());
    server.ctx.luna.stop_all();
    for p in &server.paths {
        let _ = std::fs::remove_file(p);
    }
    crate::info!("stopped");
    0
}

impl Server {
    fn start(cfg: Config) -> Result<Server, i32> {
        if let Err(e) = prepare_run_dir(&cfg.run_dir) {
            crate::error!("run directory {}: {e}", cfg.run_dir.display());
            return Err(EXIT_UNUSABLE);
        }
        let accept_path = cfg.run_dir.join("accept.sock");
        let control_path = cfg.run_dir.join("control.sock");
        if answers(&control_path) {
            crate::error!("another ssgw is running ({})", control_path.display());
            return Err(1);
        }
        for p in [&accept_path, &control_path] {
            let _ = std::fs::remove_file(p);
        }
        if cfg.check_pins
            && let Err(e) = pins::check(&cfg.lg_root, pins::PINNED)
        {
            crate::error!("LG's gateway is not the one this gateway follows: {e}");
            return Err(EXIT_UNUSABLE);
        }
        let catalog = Catalog::load(&cfg.lg_root.join(GATEWAY_DIR)).map_err(|e| {
            crate::error!("LG's interface files: {e}");
            EXIT_UNUSABLE
        })?;
        let tls = match cfg.tls {
            true => Some(load_tls(&cfg).map_err(|e| {
                crate::error!("{e}");
                EXIT_UNUSABLE
            })?),
            false => None,
        };
        if let Err(e) = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&cfg.state_dir) {
            crate::warn!("state directory {}: {e}", cfg.state_dir.display());
        }
        let store = Store::load(cfg.state_dir.join("clients.json"));
        let accept = bind(&accept_path)?;
        let control = bind(&control_path)?;
        let icons_dir = cfg.run_dir.join("icons");
        let _ = std::fs::remove_dir_all(&icons_dir);
        let ctx = Ctx {
            lg_http: cfg.lg_http.clone(),
            first_use: cfg.lg_root.join("var/luna/preferences/ran-firstuse"),
            catalog,
            store,
            luna: Luna::new(cfg.luna_send.clone()),
            resources: Resources::default(),
            hello: hello_payload(&cfg.lg_root),
            recent: VecDeque::new(),
            standby: false,
            launches: Launches::default(),
            icons: Icons::new(icons_dir),
            broker: cfg.broker.map(Broker::new),
        };
        let mut server = Server {
            ctx,
            sessions: HashMap::new(),
            next_id: 1,
            accept,
            control,
            paths: [accept_path, control_path],
            handoffs: Vec::new(),
            controls: Vec::new(),
            tls,
            started: Instant::now(),
            version: cfg.version,
            access: true,
            retry_power: None,
            retry_settings: None,
            stop_by: None,
        };
        server.watch_power();
        server.watch_settings();
        crate::info!(
            "ssgw {} ready ({}, {} stored clients)",
            server.version,
            if server.tls.is_some() { "TLS" } else { "plain" },
            server.ctx.store.len()
        );
        Ok(server)
    }

    fn watch_power(&mut self) {
        self.ctx.luna.subscribe(Owner { sess: 0, op: OP_POWER, tag: 0 }, POWER, &json!({"subscribe": true}));
    }

    fn watch_settings(&mut self) {
        let owner = Owner { sess: 0, op: OP_SETTINGS, tag: 0 };
        self.ctx.luna.cancel(owner);
        let payload = json!({"category": "network", "keys": ["allowMobileDeviceAccess"], "subscribe": true});
        self.ctx.luna.subscribe(owner, SETTINGS, &payload);
    }

    fn serve(&mut self, stop: RawFd) {
        let mut fds: Vec<libc::pollfd> = Vec::new();
        let mut slots: Vec<Slot> = Vec::new();
        let mut want: Vec<(Which, RawFd, i16)> = Vec::new();
        loop {
            let now = Instant::now();
            self.tick(now);
            self.reap();
            if let Some(by) = self.stop_by
                && (self.sessions.is_empty() || now >= by)
            {
                return;
            }

            fds.clear();
            slots.clear();
            let mut add = |fd: RawFd, events: i16, slot: Slot| {
                fds.push(libc::pollfd { fd, events, revents: 0 });
                slots.push(slot);
            };
            if self.stop_by.is_none() {
                add(stop, libc::POLLIN, Slot::Stop);
                add(self.accept.as_raw_fd(), libc::POLLIN, Slot::Accept);
            }
            add(self.control.as_raw_fd(), libc::POLLIN, Slot::Control);
            for (i, h) in self.handoffs.iter().enumerate() {
                add(h.stream.as_raw_fd(), libc::POLLIN, Slot::Handoff(i));
            }
            for (i, c) in self.controls.iter().enumerate() {
                add(c.stream.as_raw_fd(), libc::POLLIN, Slot::Ctl(i));
            }
            for (id, s) in &self.sessions {
                want.clear();
                s.fds(&mut want);
                for (which, fd, events) in &want {
                    add(*fd, *events, Slot::Sess(*id, *which));
                }
            }
            for (pid, fd) in self.ctx.luna.fds() {
                add(fd, libc::POLLIN, Slot::Luna(pid));
            }
            if let Some(b) = &self.ctx.broker
                && !b.gone
            {
                let out = if b.out.is_empty() { 0 } else { libc::POLLOUT };
                add(b.fd.as_raw_fd(), libc::POLLIN | out, Slot::Broker);
            }

            let timeout = if self.ctx.luna.has_events() {
                0
            } else {
                match self.next_deadline() {
                    Some(at) => at.saturating_duration_since(now).as_millis().min(60_000) as i32 + 1,
                    None => 60_000,
                }
            };
            // SAFETY: the pollfds live across the call.
            let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
            if n < 0 {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::Interrupted {
                    crate::error!("poll: {e}");
                    return;
                }
                continue;
            }
            let now = Instant::now();
            // descending, so removing a handoff or control connection leaves the indices still to visit intact
            for k in (0..fds.len()).rev() {
                if fds[k].revents == 0 {
                    continue;
                }
                let revents = fds[k].revents;
                match slots[k] {
                    Slot::Stop => {
                        let mut b = [0u8; 16];
                        // SAFETY: reads into a local buffer from the stop pipe.
                        unsafe { libc::read(stop, b.as_mut_ptr().cast(), b.len()) };
                        self.stop(now);
                    }
                    Slot::Accept => self.accept_handoffs(now),
                    Slot::Control => self.accept_controls(now),
                    Slot::Handoff(i) => self.take_handoff(i, now),
                    Slot::Ctl(i) => self.control_ready(i),
                    Slot::Sess(id, which) => {
                        if let Some(s) = self.sessions.get_mut(&id)
                            && !s.dead
                        {
                            s.on_ready(&mut self.ctx, which, now);
                        }
                    }
                    Slot::Luna(pid) => self.ctx.luna.readable(pid),
                    Slot::Broker => self.broker_ready(revents, now),
                }
            }
            self.dispatch_luna(now);
            for s in self.sessions.values_mut() {
                if !s.dead {
                    s.flush(&mut self.ctx, now);
                }
            }
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        let sessions = self.sessions.values().filter_map(Session::deadline);
        let handoffs = self.handoffs.iter().map(|h| h.deadline);
        let controls = self.controls.iter().map(|c| c.deadline);
        sessions
            .chain(handoffs)
            .chain(controls)
            .chain(self.ctx.luna.next_deadline())
            .chain(self.retry_power)
            .chain(self.retry_settings)
            .chain(self.stop_by)
            .min()
    }

    fn tick(&mut self, now: Instant) {
        self.ctx.luna.tick(now);
        for s in self.sessions.values_mut() {
            if !s.dead && s.deadline().is_some_and(|d| d <= now) {
                s.tick(&mut self.ctx, now);
            }
        }
        self.handoffs.retain(|h| h.deadline > now);
        self.controls.retain(|c| c.deadline > now);
        if self.retry_power.is_some_and(|t| t <= now) {
            self.retry_power = None;
            self.watch_power();
        }
        if self.retry_settings.is_some_and(|t| t <= now) {
            self.retry_settings = None;
            self.watch_settings();
        }
    }

    fn reap(&mut self) {
        let dead: Vec<u32> = self.sessions.iter().filter(|(_, s)| s.dead).map(|(id, _)| *id).collect();
        for id in dead {
            self.sessions.remove(&id);
            self.ctx.luna.cancel_session(id);
            crate::debug!("session {id} ended");
        }
    }

    fn stop(&mut self, now: Instant) {
        crate::info!("stopping");
        self.stop_by = Some(now + STOP_WAIT);
        self.handoffs.clear();
        for s in self.sessions.values_mut() {
            s.shut(&mut self.ctx, now);
        }
    }

    fn set_access(&mut self, on: bool, now: Instant) {
        if on == self.access {
            return;
        }
        self.access = on;
        if on {
            crate::info!("mobile device access turned on");
            return;
        }
        crate::info!("mobile device access turned off; closing every session");
        for s in self.sessions.values_mut() {
            s.shut(&mut self.ctx, now);
        }
    }

    fn dispatch_luna(&mut self, now: Instant) {
        // replies can start calls whose shared subscriptions answer at once; a few rounds settle them
        for _ in 0..8 {
            if !self.ctx.luna.has_events() {
                return;
            }
            for ev in self.ctx.luna.take_events() {
                let (owner, reply) = match ev {
                    Event::Reply(o, v) => (o, Some(v)),
                    Event::End(o) => (o, None),
                };
                if owner.sess == 0 {
                    self.watch_event(owner.op, reply.as_deref(), now);
                } else if let Some(s) = self.sessions.get_mut(&owner.sess)
                    && !s.dead
                {
                    s.on_luna(&mut self.ctx, owner, reply.as_deref());
                }
            }
        }
    }

    fn watch_event(&mut self, op: u32, reply: Option<&Value>, now: Instant) {
        match (op, reply) {
            (OP_POWER, Some(v)) => {
                if let Some(state) = v.get("state").and_then(Value::as_str) {
                    let standby = state == "Active Standby";
                    if standby != self.ctx.standby {
                        crate::info!("power state {state}");
                    }
                    self.ctx.standby = standby;
                }
            }
            (OP_POWER, None) => {
                crate::warn!("the power-state watch ended; retrying in {} s", WATCH_RETRY.as_secs());
                self.ctx.standby = false;
                self.retry_power = Some(now + WATCH_RETRY);
            }
            (OP_SETTINGS, Some(v)) => match v.get("settings") {
                Some(s) if truthy(Some(s)) => {
                    if let Some(on) = s.get("allowMobileDeviceAccess") {
                        self.set_access(truthy(Some(on)), now);
                    }
                    self.retry_settings = None;
                }
                // LG treats an unreadable setting as on and looks again in a minute
                _ => self.settings_unknown(now),
            },
            (OP_SETTINGS, None) => self.settings_unknown(now),
            _ => {}
        }
    }

    fn settings_unknown(&mut self, now: Instant) {
        self.set_access(true, now);
        if self.retry_settings.is_none() {
            crate::warn!("mobile device access setting unreadable; assuming on, retrying in {} s", WATCH_RETRY.as_secs());
            self.retry_settings = Some(now + WATCH_RETRY);
        }
    }

    // ---- handoffs from ssgw-accept ----

    fn accept_handoffs(&mut self, now: Instant) {
        loop {
            let stream = match self.accept.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    crate::warn!("accept socket: {e}");
                    return;
                }
            };
            let uid = net::peer_uid(stream.as_raw_fd());
            // SAFETY: geteuid has no preconditions.
            let euid = unsafe { libc::geteuid() };
            if uid != Some(0) && uid != Some(euid) {
                crate::warn!("refused a handoff from uid {uid:?}");
                continue;
            }
            if self.stop_by.is_some() || self.sessions.len() + self.handoffs.len() >= MAX_SESSIONS {
                // no ready byte: ssgw-accept hands the connection to LG's gateway
                crate::info!("{} connections open; LG's gateway takes the next", self.sessions.len());
                continue;
            }
            if stream.set_nonblocking(true).is_err() || (&stream).write(b"R").ok() != Some(1) {
                continue;
            }
            self.handoffs.push(Handoff { stream, deadline: now + HANDOFF_WAIT });
        }
    }

    fn take_handoff(&mut self, i: usize, now: Instant) {
        let got = match net::recv_fd(self.handoffs[i].stream.as_raw_fd(), 1) {
            Ok(None) => return,
            Ok(Some(r)) => r,
            Err(e) => {
                crate::debug!("handoff: {e}");
                self.handoffs.swap_remove(i);
                return;
            }
        };
        let h = self.handoffs.swap_remove(i);
        let (Some(fd), Some(&mode)) = (got.fd, got.data.first()) else { return };
        if mode != if self.tls.is_some() { b'T' } else { b'P' } {
            crate::warn!("a handoff asked for the other transport; dropped");
            return;
        }
        if (&h.stream).write(b"T").ok() != Some(1) {
            // ssgw-accept gave up waiting and closes the connection itself
            return;
        }
        if !self.access {
            crate::info!("mobile device access is off; connection closed");
            return;
        }
        match Conn::new(fd, self.tls.as_ref()) {
            Ok(conn) => {
                let id = self.next_id;
                self.next_id = self.next_id.wrapping_add(1).max(1);
                self.sessions.insert(id, Session::new(id, conn, self.tls.is_some(), now));
            }
            Err(e) => crate::warn!("could not take a connection: {e}"),
        }
    }

    fn broker_ready(&mut self, revents: i16, now: Instant) {
        let Some(b) = self.ctx.broker.as_mut() else { return };
        if revents & libc::POLLOUT != 0 {
            b.flush();
        }
        let answers = if revents & !libc::POLLOUT != 0 || b.gone { b.read() } else { Vec::new() };
        for (sess, r) in answers {
            if let Some(s) = self.sessions.get_mut(&sess)
                && !s.dead
            {
                s.on_target(&mut self.ctx, r, now);
            }
        }
    }

    // ---- control socket ----

    fn accept_controls(&mut self, now: Instant) {
        loop {
            match self.control.accept() {
                Ok((s, _)) => {
                    if s.set_nonblocking(true).is_ok() {
                        self.controls.push(Control { stream: s, buf: Vec::new(), deadline: now + CONTROL_WAIT });
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return,
            }
        }
    }

    fn control_ready(&mut self, i: usize) {
        let mut chunk = [0u8; 128];
        let c = &mut self.controls[i];
        let n = match c.stream.read(&mut chunk) {
            Ok(0) | Err(_) => {
                self.controls.swap_remove(i);
                return;
            }
            Ok(n) => n,
        };
        c.buf.extend_from_slice(&chunk[..n]);
        let Some(nl) = c.buf.iter().position(|b| *b == b'\n') else {
            if c.buf.len() > 64 {
                self.controls.swap_remove(i);
            }
            return;
        };
        let cmd = String::from_utf8_lossy(&c.buf[..nl]).trim().to_owned();
        let reply = self.control_reply(&cmd);
        let c = self.controls.swap_remove(i);
        let _ = (&c.stream).write_all(format!("{reply}\n").as_bytes());
    }

    fn control_reply(&mut self, cmd: &str) -> Value {
        match cmd {
            "ping" => {
                let open = self.sessions.values().filter(|s| !s.dead && s.is_open()).count();
                json!({
                    "ok": true,
                    "version": self.version,
                    "uptimeS": self.started.elapsed().as_secs(),
                    "sessions": open,
                    "recent": self.ctx.recent,
                    "standby": self.ctx.standby,
                    "access": self.access,
                })
            }
            "forget" => {
                let n = self.ctx.store.clear();
                crate::info!("forgot {n} stored clients");
                json!({"ok": true, "forgotten": n})
            }
            _ => json!({"ok": false, "error": "unknown command"}),
        }
    }
}

/// The run directory is created, or must be the gateway's (or root's) and not writable by others: `ssgw-accept` trusts
/// the socket in it.
fn prepare_run_dir(dir: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => std::fs::DirBuilder::new().recursive(true).mode(0o755).create(dir),
        Err(e) => Err(e),
        Ok(m) => {
            // SAFETY: geteuid has no preconditions.
            let euid = unsafe { libc::geteuid() };
            if !m.is_dir() {
                Err(io::Error::other("not a directory"))
            } else if m.uid() != euid && m.uid() != 0 {
                Err(io::Error::other(format!("owned by uid {}", m.uid())))
            } else if m.mode() & 0o022 != 0 {
                Err(io::Error::other(format!("mode {:o} lets others write", m.mode() & 0o777)))
            } else {
                Ok(())
            }
        }
    }
}

/// Whether a gateway answers on this control socket.
fn answers(path: &Path) -> bool {
    let Ok(mut s) = UnixStream::connect(path) else { return false };
    let _ = s.set_read_timeout(Some(CONTROL_WAIT));
    let _ = s.set_write_timeout(Some(CONTROL_WAIT));
    let mut buf = [0u8; 64];
    s.write_all(b"ping\n").is_ok() && matches!(s.read(&mut buf), Ok(n) if n > 0)
}

fn bind(path: &Path) -> Result<UnixListener, i32> {
    let l = UnixListener::bind(path)
        .and_then(|l| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            l.set_nonblocking(true)?;
            Ok(l)
        })
        .map_err(|e| {
            crate::error!("{}: {e}", path.display());
            1
        })?;
    Ok(l)
}

/// LG's key comes from the TV's secure store; the certificate chain (leaf and intermediate, as LG sends) from its files.
fn load_tls(cfg: &Config) -> Result<Arc<rustls::ServerConfig>, String> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let v = luna::call_blocking(&cfg.luna_send, SSL_KEY, "{}", Duration::from_secs(10))?;
    if v.get("returnValue") == Some(&Value::Bool(false)) {
        return Err(format!("getSSGSSL failed: {} {}", v.get("errorCode").unwrap_or(&Value::Null), v.get("errorText").unwrap_or(&Value::Null)));
    }
    let pem = v.get("Value").and_then(Value::as_str).ok_or("getSSGSSL answered without a key")?;
    // the error would quote the key's text
    let key = PrivateKeyDer::from_pem_slice(pem.as_bytes()).map_err(|_| "getSSGSSL's key is not a PEM private key".to_owned())?;
    let dir = cfg.lg_root.join(GATEWAY_DIR).join("certs/cert");
    let mut chain = Vec::new();
    for name in ["ssg.server.cert.pem", "ica.sub.ssl.cert.pem"] {
        let path = dir.join(name);
        chain.push(CertificateDer::from_pem_file(&path).map_err(|e| format!("{}: {e}", path.display()))?);
    }
    let mut sc = rustls::ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|e| format!("LG's certificate and key do not go together: {e}"))?;
    sc.session_storage = rustls::server::ServerSessionMemoryCache::new(32);
    Ok(Arc::new(sc))
}

/// The `hello` answer's payload, as LG's settings manager builds it at start.
fn hello_payload(root: &Path) -> Value {
    let mut p = Map::new();
    p.insert("protocolVersion".into(), json!(1));
    p.insert("deviceType".into(), json!("tv"));
    p.insert("deviceOS".into(), json!("webOS"));
    p.insert("deviceOSVersion".into(), json!("4.1.0"));
    if let Some(v) = std::fs::read_to_string(root.join("etc/starfish-release")).ok().as_deref().and_then(release_version) {
        p.insert("deviceOSReleaseVersion".into(), json!(v));
    }
    if let Ok(uuid) = std::fs::read_to_string(root.join("var/preferences/com.webos.service.ssg.ssdpmanager/uuid")) {
        p.insert("deviceUUID".into(), json!(uuid));
    }
    p.insert("pairingTypes".into(), json!(["PIN", "PROMPT", "COMBINED", "LGSWITCH-PIN"]));
    Value::Object(p)
}

/// `/(\w+)\s+release\s+([\w\.\-]+)\s+\(([\w\-]+)\)/` on `/etc/starfish-release`, the version up to its first `-`.
fn release_version(text: &str) -> Option<String> {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let words: Vec<&str> = text.split_whitespace().collect();
    (1..words.len().saturating_sub(2)).find_map(|i| {
        let (before, version, build) = (words[i - 1], words[i + 1], words[i + 2]);
        let build = build.strip_prefix('(')?;
        let close = build.find(')')?;
        let ok = words[i] == "release"
            && before.ends_with(word)
            && version.chars().all(|c| word(c) || c == '.' || c == '-')
            && close > 0
            && build[..close].chars().all(|c| word(c) || c == '-');
        ok.then(|| version.split('-').next().unwrap_or_default().to_owned())
    })
}

#[cfg(test)]
mod tests;
