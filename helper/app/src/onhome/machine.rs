//! The listener: one thread that waits for Home Assistant's LumaShow integration and keeps what it shares.
//!
//! The protocol is `lumashow-ha`'s `docs/protocol.md` and the session is `ha-bridge`'s (`Session`: the Noise handshake keyed from the
//! pairing key, `hello` and `welcome`, pings, timeouts, with no I/O); this is the socket loop around it, as `ha_bridge::Bridge` has one,
//! trimmed to the two modules Home uses (`states`, `actions`) and keeping the entities whole ([`Mirror`], which `ha-bridge`'s does not).
//! `cameras` and `notify` are not offered, so Home Assistant sends the TV neither a camera nor a notification over this link.
//!
//! **Who may connect.** Whoever holds the TV's pairing key (the Bluetooth proxy's, `ble-link`): the channel key is made from it and read
//! at every connection, so a TV that is not paired lets nobody in, and a session made with a key the TV no longer holds is ended at the
//! next check. One session is active at a time: a new one that gets as far as `hello` replaces the old. At most four connections that have
//! not finished their handshake wait, each ten seconds. **The TV never dials out and holds no Home Assistant address or token.**
//!
//! **It costs nothing when it is not started**: no thread, no socket. While it runs the thread sleeps in `poll` on the listener, the
//! connection and one eventfd, with the next time anything is due as the timeout.

use super::mirror::{Mirror, Snap};
use ha_bridge::session::{Event, Session, Timing};
use ha_bridge::wire::{self, ClientInfo, DeviceInfo, Message};
use parking_lot::Mutex;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const MAX_WAITING: usize = 4;
const MAX_PENDING: usize = 1 << 20;
const RESYNC_EVERY: Duration = Duration::from_secs(5);
const GONE: &str = "Home Assistant's LumaShow integration disconnected";
const NOT_CONNECTED: &str = "Home Assistant's LumaShow integration is not connected";

/// The TV's pairing key now (`None`: not paired, nobody is let in).
pub type KeySource = Arc<dyn Fn() -> Option<[u8; 32]> + Send + Sync>;
pub type Log = Arc<dyn Fn(String) + Send + Sync>;
/// Told (on the listener's thread, with nothing locked) that what the mirror or the link says has changed.
pub type Changed = Arc<dyn Fn() + Send + Sync>;
/// Called once with how an action ended: `Ok`, or why not in words.
pub type Done = Box<dyn FnOnce(Result<(), String>) + Send>;

#[derive(Clone)]
pub struct Config {
    pub bind: String,
    pub key: KeySource,
    pub device: DeviceInfo,
    pub timing: Timing,
    pub log: Log,
    pub changed: Changed,
}

/// An action: services of one domain on named entities, as the protocol's `actions.call` carries them.
pub struct Call {
    pub entity_ids: Vec<String>,
    pub verb: String,
    pub args: Map<String, Value>,
    /// A person at the TV's own screen confirmed this (`origin: touch`, `confirmed: true`): the one way a sensitive service is asked for.
    /// Only `OnHome::unlock` sets it, after the shell's own question.
    pub touch: bool,
}

/// Whether `verb` on `entity` moves a door, a gate or an alarm, or switches a camera or a siren: never asked for unless a person at the TV
/// confirmed ([`Call::touch`]), whatever the mirror says the integration listed. (The mirror holds no entity of these domains but locks.)
fn sensitive(entity: &str, verb: &str) -> bool {
    match entity.split('.').next().unwrap_or("") {
        "lock" => matches!(verb, "unlock" | "open"),
        "alarm_control_panel" | "siren" | "camera" | "valve" => true,
        _ => false,
    }
}

/// What the readers see.
#[derive(Default)]
pub struct Shared {
    pub mirror: Mirror,
    pub client: Option<ClientInfo>,
    pub refused: u64,
    pub sessions: u64,
}

struct Wake(OwnedFd);

impl Wake {
    fn new() -> io::Result<Self> {
        // SAFETY: eventfd with flags; the descriptor it returns is owned from here on.
        let fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a descriptor eventfd just made.
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    fn wake(&self) {
        let one = 1u64.to_ne_bytes();
        // SAFETY: eight bytes to the eventfd; a full counter (EAGAIN) means it is already woken.
        unsafe { libc::write(self.0.as_raw_fd(), one.as_ptr().cast(), one.len()) };
    }

    fn drain(&self) {
        let mut buf = [0u8; 8];
        // SAFETY: eight bytes from the eventfd.
        unsafe { libc::read(self.0.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
    }

    fn fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

enum Command {
    Call(Call, Done),
    Recheck,
    Stop,
}

/// The handle of a running listener. Dropping it stops it (without waiting for the thread).
pub struct Bridge {
    commands: Sender<Command>,
    wake: Arc<Wake>,
    stopped: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
    addr: std::net::SocketAddr,
    thread: Mutex<Option<JoinHandle<()>>>,
}

struct Conn {
    stream: TcpStream,
    session: Session,
    out: Vec<u8>,
    peer: String,
    closing: bool,
    dead: bool,
    counted: bool,
}

struct Pending {
    done: Done,
    due: Instant,
}

struct Machine {
    cfg: Config,
    listener: TcpListener,
    inbox: Receiver<Command>,
    wake: Arc<Wake>,
    stopped: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
    conns: HashMap<u64, Conn>,
    next_conn: u64,
    active: Option<u64>,
    pending: HashMap<u64, Pending>,
    next_op: u64,
    dirty: bool,
    last_told: Instant,
    last_key_check: Instant,
    last_resync: Option<Instant>,
}

/// The least time between two tellings of a change (a burst of updates is one).
const TELL_EVERY: Duration = Duration::from_millis(250);

impl Bridge {
    pub fn start(config: Config) -> io::Result<Self> {
        let listener = TcpListener::bind(config.bind.as_str())?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let (commands, inbox) = channel();
        let wake = Arc::new(Wake::new()?);
        let stopped = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(Shared::default()));
        let machine = Machine {
            cfg: config,
            listener,
            inbox,
            wake: wake.clone(),
            stopped: stopped.clone(),
            shared: shared.clone(),
            conns: HashMap::new(),
            next_conn: 1,
            active: None,
            pending: HashMap::new(),
            next_op: 1,
            dirty: false,
            last_told: Instant::now(),
            last_key_check: Instant::now(),
            last_resync: None,
        };
        let thread = std::thread::Builder::new().name("onhome-bridge".into()).spawn(move || machine.run())?;
        Ok(Self { commands, wake, stopped, shared, addr, thread: Mutex::new(Some(thread)) })
    }

    fn send(&self, command: Command) -> Result<(), Box<Command>> {
        match self.commands.send(command) {
            Ok(()) => {
                self.wake.wake();
                Ok(())
            }
            Err(e) => Err(Box::new(e.0)),
        }
    }

    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    /// Reads the mirror and the link under one lock.
    pub fn with<R>(&self, f: impl FnOnce(&Shared) -> R) -> R {
        f(&self.shared.lock())
    }

    /// An action the integration decides: `done` is called once.
    pub fn call(&self, call: Call, done: Done) {
        if let Err(command) = self.send(Command::Call(call, done))
            && let Command::Call(_, done) = *command
        {
            done(Err("the listener is closed".into()));
        }
    }

    /// The key may have changed (the Pair window): sessions made with another one end.
    pub fn recheck_key(&self) {
        let _ = self.send(Command::Recheck);
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let _ = self.commands.send(Command::Stop);
        self.wake.wake();
    }

    pub fn stop_and_wait(&self) {
        self.stop();
        if let Some(thread) = self.thread.lock().take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop();
    }
}

fn keepalive(stream: &TcpStream) {
    let fd = stream.as_raw_fd();
    // A Home Assistant that vanished (power, Wi-Fi) is noticed by the kernel in about 90 seconds, with no timer of ours.
    for (level, name, value) in [(libc::SOL_SOCKET, libc::SO_KEEPALIVE, 1), (libc::IPPROTO_TCP, libc::TCP_KEEPIDLE, 60), (libc::IPPROTO_TCP, libc::TCP_KEEPINTVL, 10), (libc::IPPROTO_TCP, libc::TCP_KEEPCNT, 3)] {
        // SAFETY: setsockopt on a live descriptor with an int.
        unsafe {
            libc::setsockopt(fd, level, name, (&raw const value).cast(), std::mem::size_of::<libc::c_int>() as libc::socklen_t);
        }
    }
}

fn urandom(buf: &mut [u8]) {
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(buf);
    }
}

/// The modules the TV speaks: `states` and `actions` only.
fn offered() -> BTreeMap<String, u32> {
    wire::module::BUILT_IN.iter().filter(|(name, _)| matches!(*name, wire::module::STATES | wire::module::ACTIONS)).map(|(n, v)| ((*n).to_owned(), *v)).collect()
}

impl Machine {
    fn log(&self, line: String) {
        (self.cfg.log)(line);
    }

    fn run(mut self) {
        loop {
            if self.stopped.load(Ordering::SeqCst) || !self.drain_commands() {
                break;
            }
            let now = Instant::now();
            self.timers(now);
            self.flush_all();
            self.reap();
            self.wait(now);
            if self.stopped.load(Ordering::SeqCst) {
                break;
            }
            let now = Instant::now();
            self.accept(now);
            self.read_all(now);
            self.flush_all();
            self.reap();
        }
        self.finish();
    }

    fn finish(&mut self) {
        if let Some(id) = self.active
            && let Some(conn) = self.conns.get_mut(&id)
        {
            conn.session.say_bye("shutdown");
        }
        self.flush_all();
        self.conns.clear();
        for (_, call) in self.pending.drain() {
            (call.done)(Err(GONE.into()));
        }
        {
            let mut shared = self.shared.lock();
            shared.client = None;
            shared.mirror.clear();
        }
        (self.cfg.changed)();
    }

    fn tell(&mut self, now: Instant) {
        self.dirty = true;
        self.tell_due(now);
    }

    fn tell_due(&mut self, now: Instant) {
        if self.dirty && now.duration_since(self.last_told) >= TELL_EVERY {
            self.dirty = false;
            self.last_told = now;
            (self.cfg.changed)();
        }
    }

    fn next_due(&self, now: Instant) -> Option<Instant> {
        let mut due: Vec<Instant> = self.conns.values().filter_map(|c| c.session.deadline(&self.cfg.timing)).collect();
        due.extend(self.pending.values().map(|c| c.due));
        if let Some(since) = self.shared.lock().mirror.partial_since() {
            due.push(since + self.cfg.timing.partial_within);
        }
        if self.dirty {
            due.push(self.last_told + TELL_EVERY);
        }
        if !self.conns.is_empty() {
            due.push(self.last_key_check + self.cfg.timing.key_check);
        }
        due.into_iter().min().map(|at| at.max(now))
    }

    fn timers(&mut self, now: Instant) {
        let late: Vec<(u64, &'static str)> = self.conns.iter().filter_map(|(id, c)| Some((*id, c.session.overdue(now, &self.cfg.timing)?))).collect();
        for (id, why) in late {
            let Some(conn) = self.conns.get_mut(&id) else { continue };
            let peer = conn.peer.clone();
            let unfinished = !conn.session.is_active() && !conn.counted;
            conn.counted |= unfinished;
            conn.dead = true;
            if unfinished {
                self.shared.lock().refused += 1;
            }
            self.log(format!("the integration's connection from {peer} ended: {why}"));
        }
        let expired: Vec<u64> = self.pending.iter().filter(|(_, c)| c.due <= now).map(|(op, _)| *op).collect();
        for op in expired {
            if let Some(call) = self.pending.remove(&op) {
                (call.done)(Err(wire::words("timeout").into()));
            }
        }
        if self.shared.lock().mirror.expire_partial(now, self.cfg.timing.partial_within) {
            self.log("half a snapshot was dropped: the rest did not come".into());
            self.resync(now);
        }
        self.tell_due(now);
        if now.duration_since(self.last_key_check) >= self.cfg.timing.key_check {
            self.check_keys(now);
        }
    }

    fn check_keys(&mut self, now: Instant) {
        self.last_key_check = now;
        let key = (self.cfg.key)();
        for conn in self.conns.values_mut() {
            if key != Some(*conn.session.pairing()) && !conn.closing {
                conn.session.say_bye("unpaired");
                conn.closing = true;
            }
        }
    }

    fn wait(&mut self, now: Instant) {
        let mut fds: Vec<libc::pollfd> = vec![libc::pollfd { fd: self.wake.fd(), events: libc::POLLIN, revents: 0 }, libc::pollfd { fd: self.listener.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        for conn in self.conns.values() {
            let mut events = libc::POLLIN;
            if !conn.out.is_empty() {
                events |= libc::POLLOUT;
            }
            fds.push(libc::pollfd { fd: conn.stream.as_raw_fd(), events, revents: 0 });
        }
        let timeout = match self.next_due(now) {
            Some(at) => i32::try_from(at.duration_since(now).as_millis() + 1).unwrap_or(i32::MAX),
            None => -1,
        };
        // SAFETY: a slice of live pollfds.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if fds[0].revents != 0 {
            self.wake.drain();
        }
    }

    fn drain_commands(&mut self) -> bool {
        loop {
            match self.inbox.try_recv() {
                Ok(Command::Stop) => return false,
                Ok(Command::Call(call, done)) => self.start_call(call, done, Instant::now()),
                Ok(Command::Recheck) => self.check_keys(Instant::now()),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn send_active(&mut self, message: &Message, module: &str) -> bool {
        let Some(id) = self.active else { return false };
        let Some(conn) = self.conns.get_mut(&id) else { return false };
        conn.session.carries(module) && conn.session.send(message)
    }

    fn resync(&mut self, now: Instant) {
        if self.last_resync.is_some_and(|at| now.duration_since(at) < RESYNC_EVERY) {
            return;
        }
        self.last_resync = Some(now);
        self.send_active(&Message::Resync, wire::module::STATES);
    }

    fn start_call(&mut self, call: Call, done: Done, now: Instant) {
        let Some(id) = self.active else {
            return done(Err(NOT_CONNECTED.into()));
        };
        if !self.conns.get(&id).is_some_and(|c| c.session.carries(wire::module::ACTIONS)) {
            return done(Err(NOT_CONNECTED.into()));
        }
        if call.entity_ids.is_empty() || call.entity_ids.len() > wire::MAX_CALL_ENTITIES {
            return done(Err(wire::words("too_many").into()));
        }
        {
            // what the integration said the TV may do is the only thing the TV does; a sensitive service only for a confirmed touch, and only
            // the lock's own `unlock`
            let shared = self.shared.lock();
            for entity in &call.entity_ids {
                let sensitive = sensitive(entity, &call.verb);
                match shared.mirror.get(entity) {
                    _ if sensitive && !call.touch => {
                        drop(shared);
                        return done(Err(wire::words("needs_touch").into()));
                    }
                    Some(item) if call.touch => {
                        let ok = entity.starts_with("lock.") && call.verb == "unlock" && (item.ctl_pin.contains(&call.verb) || item.ctl.contains(&call.verb));
                        if !ok {
                            drop(shared);
                            return done(Err(wire::words("not_allowed").into()));
                        }
                    }
                    Some(item) if item.ctl.contains(&call.verb) => {}
                    Some(_) => {
                        drop(shared);
                        return done(Err(wire::words("not_allowed").into()));
                    }
                    None => {
                        drop(shared);
                        return done(Err(format!("{entity} is not shared with this TV")));
                    }
                }
            }
        }
        let op = self.next_op;
        self.next_op += 1;
        let message = Message::Call(wire::Call { op, entity_ids: call.entity_ids, verb: call.verb, args: call.args, origin: if call.touch { "touch" } else { "plugin" }.into(), confirmed: call.touch.then_some(true), pin: None });
        if !self.send_active(&message, wire::module::ACTIONS) {
            return done(Err("the action could not be sent".into()));
        }
        self.pending.insert(op, Pending { done, due: now + self.cfg.timing.call_within });
    }

    fn accept(&mut self, now: Instant) {
        loop {
            let (stream, peer) = match self.listener.accept() {
                Ok(accepted) => accepted,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            };
            // A TV that is not paired has no key and lets nobody in.
            let Some(key) = (self.cfg.key)() else { continue };
            let waiting = self.conns.keys().filter(|id| Some(**id) != self.active).count();
            if waiting >= MAX_WAITING || stream.set_nonblocking(true).is_err() || stream.set_nodelay(true).is_err() {
                continue;
            }
            keepalive(&stream);
            let mut ephemeral = [0u8; 32];
            urandom(&mut ephemeral);
            let id = self.next_conn;
            self.next_conn += 1;
            self.conns.insert(id, Conn { stream, session: Session::new(key, ephemeral, self.cfg.device.clone(), offered(), now), out: Vec::new(), peer: peer.to_string(), closing: false, dead: false, counted: false });
        }
    }

    fn read_all(&mut self, now: Instant) {
        let ids: Vec<u64> = self.conns.keys().copied().collect();
        for id in ids {
            let mut events = Vec::new();
            {
                let Some(conn) = self.conns.get_mut(&id) else { continue };
                let mut buf = [0u8; 4096];
                while !conn.dead && !conn.closing {
                    match conn.stream.read(&mut buf) {
                        Ok(0) => conn.dead = true,
                        Ok(n) => events.extend(conn.session.feed(&buf[..n], now)),
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        Err(_) => conn.dead = true,
                    }
                }
            }
            for event in events {
                self.event(id, event, now);
            }
        }
    }

    fn event(&mut self, id: u64, event: Event, now: Instant) {
        match event {
            Event::Active { client, .. } => self.activate(id, client, now),
            Event::Message(message) => {
                if self.active == Some(id) {
                    self.message(message, now);
                }
            }
            Event::Binary { .. } => {}
            Event::Closing(why) => {
                let Some(conn) = self.conns.get_mut(&id) else { return };
                conn.closing = true;
                let (peer, refused, counted) = (conn.peer.clone(), conn.session.was_refused(), conn.counted);
                if refused && !counted {
                    conn.counted = true;
                    self.shared.lock().refused += 1;
                }
                self.log(format!("the integration's connection from {peer} ended: {why}"));
            }
        }
    }

    fn activate(&mut self, id: u64, client: ClientInfo, now: Instant) {
        if let Some(old) = self.active
            && old != id
        {
            if let Some(conn) = self.conns.get_mut(&old) {
                conn.session.say_bye("replaced");
                conn.closing = true;
            }
            self.end_active(now);
        }
        self.active = Some(id);
        {
            let mut shared = self.shared.lock();
            shared.client = Some(client.clone());
            shared.sessions += 1;
        }
        self.log(format!("the LumaShow integration connected ({} {}, Home Assistant {})", client.name, client.version, client.ha_version));
        self.tell(now);
    }

    /// The active session is over: everything it gave the TV is dropped and said to be.
    fn end_active(&mut self, now: Instant) {
        if self.active.take().is_none() {
            return;
        }
        {
            let mut shared = self.shared.lock();
            shared.client = None;
            shared.mirror.clear();
        }
        for (_, call) in self.pending.drain() {
            (call.done)(Err(GONE.into()));
        }
        self.tell(now);
    }

    fn message(&mut self, message: Message, now: Instant) {
        match message {
            Message::Snapshot(part) => {
                let outcome = self.shared.lock().mirror.snapshot(part, now);
                match outcome {
                    Snap::Waiting => {}
                    Snap::Applied => self.tell(now),
                    Snap::Refused(why) => {
                        self.log(format!("a snapshot was refused: {why}"));
                        self.resync(now);
                    }
                }
            }
            Message::Update(update) => {
                if self.shared.lock().mirror.update(update) {
                    self.tell(now);
                }
            }
            Message::CallResult(result) => {
                if let Some(call) = self.pending.remove(&result.op) {
                    let answer = if result.ok { Ok(()) } else { Err(result.message.filter(|m| !m.is_empty()).unwrap_or_else(|| wire::words(result.code.as_deref().unwrap_or("")).to_owned())) };
                    (call.done)(answer);
                }
            }
            Message::Error { code, message } => self.log(format!("the integration reported an error: {code}: {message}")),
            // the rest is either the TV's to send or a module the TV did not offer: nothing
            _ => {}
        }
    }

    fn flush_all(&mut self) {
        for conn in self.conns.values_mut() {
            conn.out.extend(conn.session.take_output());
            if conn.session.is_closed() {
                conn.closing = true;
            }
            while !conn.out.is_empty() && !conn.dead {
                match conn.stream.write(&conn.out) {
                    Ok(0) => conn.dead = true,
                    Ok(n) => {
                        conn.out.drain(..n);
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => conn.dead = true,
                }
            }
            if conn.out.len() > MAX_PENDING {
                conn.dead = true;
            }
        }
    }

    fn reap(&mut self) {
        let gone: Vec<u64> = self.conns.iter().filter(|(_, c)| c.dead || (c.closing && c.out.is_empty())).map(|(id, _)| *id).collect();
        for id in gone {
            let conn = self.conns.remove(&id);
            if self.active == Some(id) {
                if let Some(conn) = &conn {
                    self.log(format!("the LumaShow integration disconnected ({})", conn.peer));
                }
                self.end_active(Instant::now());
            }
        }
    }
}
