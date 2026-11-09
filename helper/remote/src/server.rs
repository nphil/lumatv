//! The TV's side. One TCP listener and one UDP socket on the same port of the TV's private address; a thread accepts, one reads
//! datagrams and keeps time (the pairing window, silent sessions, held keys, state pushed to phones), and each connection has its own.
//! Nothing answers before a handshake proves the phone's key: a bad preamble, a failed handshake, a datagram without a live session and a
//! valid tag all go without a byte back.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};

use esphome_api::base64;
use esphome_api::noise::{CipherState, Responder, Transport};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Map, Value, json};

use crate::datagram::{self, Accumulator, Event, Header, Pointer};
use crate::link::{self, PairLink};
use crate::pairing::{self, Window};
use crate::proto::{self, Act, Edit, List, Press, Refusal, Request};
use crate::store::{Device, Store};
use crate::wire::{self, Mode, PREAMBLE_LEN};
use crate::{Fault, hex, unix_ms};

/// The handshake and `hello` within this of connecting.
pub const HANDSHAKE_WITHIN: Duration = Duration::from_secs(3);
/// A session silent on both channels this long ends.
pub const SILENCE: Duration = Duration::from_secs(6);
/// A key held `down` this long is released.
pub const HELD_KEY: Duration = Duration::from_secs(30);
/// A `hold`, longer than the remote's own: the compositor reads 1 s as a hold, and 2 s for Mute.
pub const HOLD: Duration = Duration::from_millis(1100);
pub const HOLD_MUTE: Duration = Duration::from_millis(2100);
pub const FRAMES_PER_SECOND: u32 = 60;
pub const DATAGRAMS_PER_SECOND: u32 = 250;
pub const MAX_SESSIONS: usize = 4;
/// Failed handshakes from one address within [`FAILURE_WINDOW`] before it is ignored.
pub const FAILURES: usize = 8;
pub const FAILURE_WINDOW: Duration = Duration::from_secs(60);
/// Connections still in their handshake; more are closed at once.
const CONNECTING: usize = 16;
const TICK: Duration = Duration::from_millis(250);
/// The phone sends `pair` right after the handshake; a person may be typing a name.
const PAIR_FRAME_WITHIN: Duration = Duration::from_secs(10);
/// A phone that stops reading cannot hold a writer longer than this.
const WRITE_WITHIN: Duration = Duration::from_secs(2);
/// Stopping waits this long for connections to let go of what their phones held.
const DRAIN_WITHIN: Duration = Duration::from_secs(3);
/// A `list` page's items in bytes of JSON, which leaves the frame's other members room within [`wire::MAX_JSON`].
const PAGE_BYTES: usize = 7000;
const MUTE: u32 = 113;

/// The TV, as the engine drives it. Calls come from the server's threads, one session's in order.
pub trait Host: Send + Sync + 'static {
    /// A key of the remote: `Some(true)` pressed, `Some(false)` released, `None` a tap.
    fn key(&self, code: u32, down: Option<bool>) -> Result<(), Fault>;
    /// Text into the active field. Never logged.
    fn text(&self, text: &str) -> Result<(), Fault>;
    fn edit(&self, edit: Edit) -> Result<(), Fault>;
    /// What one pointer datagram did, in order, and whether the phone is pointing: called for every accepted datagram that did something
    /// and every one while engaged, so the host can keep the pointer shown.
    fn pointer(&self, events: &[Event], engaged: bool);
    fn act(&self, act: &Act) -> Result<(), Fault>;
    fn list(&self, what: List) -> Result<Vec<Value>, Fault>;
    /// Every state member (`power`, `fg`, `ime`, `volume`, `acts`): read when a session opens and every 250 ms while one is live; phones
    /// get the members that changed.
    fn state(&self) -> Map<String, Value>;
    /// The theme's accent as `#rrggbb`, for `welcome`.
    fn accent(&self) -> String;
    /// The pairing window, the paired phones or the live sessions changed: [`Server::status`] and [`Server::txt`] have the new picture.
    /// Called with no lock of the server's held.
    fn changed(&self);
}

pub struct Config {
    /// The TV's private address: the listener binds it, and the pairing link names it.
    pub bind: Ipv4Addr,
    pub port: u16,
    pub name: String,
    /// The wired MAC, for Wake-on-LAN.
    pub mac: Option<[u8; 6]>,
    pub store: PathBuf,
}

pub struct Server {
    inner: Arc<Inner>,
    threads: Vec<JoinHandle<()>>,
}

struct Open {
    window: Window,
    expires: SystemTime,
    /// Which window a pairing connection claimed, so it cannot spend a try of the next one.
    serial: u64,
}

struct Inner {
    host: Arc<dyn Host>,
    name: String,
    mac: Option<[u8; 6]>,
    addr: SocketAddr,
    tv: [u8; 8],
    random: SystemRandom,
    store: Mutex<Store>,
    window: Mutex<Option<Open>>,
    serial: AtomicU64,
    sessions: Mutex<Vec<Arc<Session>>>,
    failures: Mutex<HashMap<IpAddr, Vec<Instant>>>,
    connecting: AtomicUsize,
    /// Connection threads alive, for stopping.
    connections: Mutex<usize>,
    drained: Condvar,
    udp: UdpSocket,
    stop: AtomicBool,
    standby: AtomicBool,
}

struct Session {
    udp_id: u32,
    udp_key: datagram::Key,
    device: String,
    name: String,
    /// For shutting the connection down from another thread.
    stream: TcpStream,
    out: Mutex<Out>,
    live: Mutex<Live>,
}

struct Out {
    stream: TcpStream,
    send: CipherState,
    broken: bool,
}

struct Live {
    heard: Instant,
    /// Keys down and when they are let go.
    held: Vec<(u32, Instant)>,
    pointer: Accumulator,
    seq_in: u32,
    seq_out: u32,
    frames: Meter,
    datagrams: Meter,
    /// The state as the phone last got it.
    sent: Map<String, Value>,
    ended: bool,
}

/// Counts per second.
struct Meter {
    since: Instant,
    count: u32,
}

impl Meter {
    fn new(now: Instant) -> Self {
        Meter { since: now, count: 0 }
    }

    fn allow(&mut self, now: Instant, limit: u32) -> bool {
        if now.duration_since(self.since) >= Duration::from_secs(1) {
            self.since = now;
            self.count = 0;
        }
        self.count += 1;
        self.count <= limit
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Private IPv4 and loopback: the home network, never the internet.
fn allowed(ip: IpAddr) -> bool {
    matches!(ip, IpAddr::V4(v4) if v4.is_private() || v4.is_loopback())
}

fn write_out(out: &mut Out, msg: &Value) -> bool {
    if out.broken {
        return false;
    }
    let Out { stream, send, broken } = out;
    *broken = wire::write(stream, send, msg.to_string().as_bytes()).is_err();
    !*broken
}

impl Session {
    fn send(&self, msg: &Value) -> bool {
        let sent = write_out(&mut lock(&self.out), msg);
        if !sent {
            self.end();
        }
        sent
    }

    /// Ends the connection: its reader wakes, and its thread lets go of what the phone held.
    fn end(&self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl Server {
    /// Opens the store (making the TV id the first time), binds the TCP and UDP sockets and starts serving.
    pub fn start(config: Config, host: Arc<dyn Host>) -> io::Result<Server> {
        let random = SystemRandom::new();
        let store = Store::open(&config.store, &random)?;
        let listener = TcpListener::bind((config.bind, config.port))?;
        let addr = listener.local_addr()?;
        let udp = UdpSocket::bind(addr)?;
        udp.set_read_timeout(Some(TICK))?;
        let inner = Arc::new(Inner {
            host,
            name: config.name,
            mac: config.mac,
            addr,
            tv: store.tv(),
            random,
            store: Mutex::new(store),
            window: Mutex::new(None),
            serial: AtomicU64::new(0),
            sessions: Mutex::new(Vec::new()),
            failures: Mutex::new(HashMap::new()),
            connecting: AtomicUsize::new(0),
            connections: Mutex::new(0),
            drained: Condvar::new(),
            udp,
            stop: AtomicBool::new(false),
            standby: AtomicBool::new(false),
        });
        let accepting = inner.clone();
        let tcp = thread::Builder::new().name("lumaremote-tcp".into()).spawn(move || accepting.accept(listener))?;
        let ticking = inner.clone();
        let udp = thread::Builder::new().name("lumaremote-udp".into()).spawn(move || ticking.datagrams())?;
        log::info!(target: "lumaremote", "listening on {addr}");
        Ok(Server { inner, threads: vec![tcp, udp] })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.inner.addr
    }

    pub fn tv(&self) -> [u8; 8] {
        self.inner.tv
    }

    /// `{"pairing": {"code", "qr", "expires"} | null, "devices": [{"id", "name", "model", "last", "live"}]}`: `code` as shown, `qr` the
    /// pairing link, `expires` and `last` Unix ms.
    pub fn status(&self) -> Value {
        let inner = &self.inner;
        let pairing = lock(&inner.window).as_ref().map(|open| {
            json!({"code": pairing::display(open.window.code()), "qr": inner.link(open.window.code()), "expires": unix_ms(open.expires)})
        });
        let live: Vec<String> = lock(&inner.sessions).iter().map(|s| s.device.clone()).collect();
        let devices: Vec<Value> = lock(&inner.store)
            .devices()
            .iter()
            .map(|d| json!({"id": d.id, "name": d.name, "model": d.model, "last": d.last, "live": live.contains(&d.id)}))
            .collect();
        json!({"pairing": pairing, "devices": devices})
    }

    /// The mDNS TXT record as it stands.
    pub fn txt(&self) -> Vec<(String, String)> {
        let inner = &self.inner;
        link::txt(&inner.tv, &inner.name, inner.mac, lock(&inner.window).is_some())
    }

    /// Whether a phone has a live session.
    pub fn live(&self) -> bool {
        !lock(&self.inner.sessions).is_empty()
    }

    /// Opens a pairing window with a new code (one already open is replaced) and returns the `pairing` member of [`Server::status`]. The
    /// owner's consent is the host's to ask for first.
    pub fn open_pairing(&self) -> Result<Value, &'static str> {
        let inner = &self.inner;
        if lock(&inner.store).full() {
            return Err("16 phones are paired: remove one to pair another");
        }
        let window = Window::new(pairing::new_code(&inner.random), &inner.tv, Instant::now());
        let serial = inner.serial.fetch_add(1, Ordering::SeqCst) + 1;
        *lock(&inner.window) = Some(Open { window, expires: SystemTime::now() + pairing::WINDOW, serial });
        inner.host.changed();
        Ok(self.status()["pairing"].clone())
    }

    pub fn close_pairing(&self) {
        if lock(&self.inner.window).take().is_some() {
            self.inner.host.changed();
        }
    }

    /// Removes one phone: its key goes, and a live session is told `revoked` and ended.
    pub fn forget(&self, id: &str) -> io::Result<bool> {
        let removed = lock(&self.inner.store).remove(id)?;
        if removed {
            self.inner.revoke(|s| s.device == id);
            self.inner.host.changed();
        }
        Ok(removed)
    }

    pub fn forget_all(&self) -> io::Result<()> {
        lock(&self.inner.store).clear()?;
        self.inner.revoke(|_| true);
        self.inner.host.changed();
        Ok(())
    }
}

impl Drop for Server {
    /// Stops listening, ends every session and waits for their threads to release what the phones held.
    fn drop(&mut self) {
        let inner = &self.inner;
        inner.stop.store(true, Ordering::SeqCst);
        lock(&inner.window).take();
        // The accept is woken by a connection of the server's own, which it sees after the flag.
        let _ = TcpStream::connect_timeout(&inner.addr, Duration::from_secs(1));
        for session in lock(&inner.sessions).iter() {
            session.end();
        }
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        let left = lock(&inner.connections);
        let _ = inner.drained.wait_timeout_while(left, DRAIN_WITHIN, |n| *n > 0);
        log::info!(target: "lumaremote", "stopped");
    }
}

/// Reads exactly `buf` by `deadline`, however slowly the bytes come.
fn read_by(stream: &mut TcpStream, buf: &mut [u8], deadline: Instant) -> io::Result<()> {
    let mut got = 0;
    while got < buf.len() {
        let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or(io::ErrorKind::TimedOut)?;
        stream.set_read_timeout(Some(left))?;
        match stream.read(&mut buf[got..]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The phone's `pair`: a name of 1–40 characters, and a model and an app.
fn pair_request(frame: &[u8]) -> Option<(String, String, String)> {
    let v: Value = serde_json::from_slice(frame).ok()?;
    if v.get("t")?.as_str()? != "pair" {
        return None;
    }
    let name = v.get("name")?.as_str()?.trim();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
        return None;
    }
    let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").trim().chars().filter(|c| !c.is_control()).take(64).collect();
    Some((name.to_string(), text("model"), text("app")))
}

/// Items in pages of at most [`PAGE_BYTES`] of JSON; an empty list is one empty page.
fn pages(items: Vec<Value>) -> Vec<Vec<Value>> {
    let mut pages = vec![Vec::new()];
    let mut bytes = 0;
    for item in items {
        let len = item.to_string().len() + 1;
        let page = pages.last_mut().expect("one page at least");
        if !page.is_empty() && bytes + len > PAGE_BYTES {
            pages.push(Vec::new());
            bytes = 0;
        }
        bytes += len;
        pages.last_mut().expect("one page at least").push(item);
    }
    pages
}

fn t(request: &Request) -> &'static str {
    match request {
        Request::Hello { .. } => "hello",
        Request::Key { .. } => "key",
        Request::Text(_) => "text",
        Request::Edit(_) => "edit",
        Request::Act { .. } => "act",
        Request::Get { .. } => "get",
        Request::Ping(_) => "ping",
        Request::Bye => "bye",
    }
}

fn refuse(fault: Fault, request: &Request) -> Value {
    let id = match request {
        Request::Act { id, .. } | Request::Get { id, .. } => Some(*id),
        _ => None,
    };
    Refusal { fault, about: t(request).into(), id }.reply()
}

enum PairError {
    Handshake,
    Refused(String),
}

type Opened = (Arc<Session>, TcpStream, wire::Reader, CipherState);

impl Inner {
    fn secret(&self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        self.random.fill(&mut bytes).expect("system random source");
        bytes
    }

    fn link(&self, code: &str) -> String {
        PairLink { host: self.addr.ip().to_string(), port: self.addr.port(), tv: self.tv, code: code.to_string() }.to_url()
    }

    fn banned(&self, ip: IpAddr, now: Instant) -> bool {
        let mut failures = lock(&self.failures);
        let Some(times) = failures.get_mut(&ip) else { return false };
        times.retain(|t| now.duration_since(*t) < FAILURE_WINDOW);
        let banned = times.len() >= FAILURES;
        if times.is_empty() {
            failures.remove(&ip);
        }
        banned
    }

    fn failed(&self, ip: IpAddr) {
        let now = Instant::now();
        let mut failures = lock(&self.failures);
        // Addresses whose failures have all aged out go, so the table cannot grow without bound.
        failures.retain(|_, times| times.last().is_some_and(|t| now.duration_since(*t) < FAILURE_WINDOW));
        failures.entry(ip).or_default().push(now);
    }

    fn revoke(&self, which: impl Fn(&Session) -> bool) {
        let gone: Vec<Arc<Session>> = lock(&self.sessions).iter().filter(|s| which(s)).cloned().collect();
        for session in gone {
            log::info!(target: "lumaremote", "{} ({}) was removed; its session ends", session.name, session.device);
            session.send(&json!({"t": "revoked"}));
            session.end();
        }
    }

    fn read_state(&self) -> Map<String, Value> {
        let state = self.host.state();
        self.standby.store(state.get("power").and_then(Value::as_str) == Some("standby"), Ordering::SeqCst);
        state
    }

    fn accept(self: Arc<Self>, listener: TcpListener) {
        for stream in listener.incoming() {
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else {
                // Out of descriptors, for one: wait rather than spin.
                thread::sleep(TICK);
                continue;
            };
            let Ok(peer) = stream.peer_addr() else { continue };
            if !allowed(peer.ip()) || self.banned(peer.ip(), Instant::now()) {
                continue;
            }
            if self.connecting.fetch_add(1, Ordering::SeqCst) >= CONNECTING {
                self.connecting.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            *lock(&self.connections) += 1;
            let me = self.clone();
            if thread::Builder::new().name("lumaremote-conn".into()).spawn(move || me.connection(stream, peer)).is_err() {
                self.connecting.fetch_sub(1, Ordering::SeqCst);
                self.left();
            }
        }
    }

    fn left(&self) {
        *lock(&self.connections) -= 1;
        self.drained.notify_all();
    }

    fn connection(self: Arc<Self>, mut stream: TcpStream, peer: SocketAddr) {
        let deadline = Instant::now() + HANDSHAKE_WITHIN;
        let _ = stream.set_nodelay(true);
        let mut first = [0u8; PREAMBLE_LEN + wire::HANDSHAKE_LEN];
        let mode = read_by(&mut stream, &mut first, deadline).ok().and_then(|()| Mode::parse(first[..PREAMBLE_LEN].try_into().expect("6 bytes")));
        let opened = match mode {
            None => {
                self.failed(peer.ip());
                None
            }
            Some(Mode::Pair) => {
                self.pair(stream, peer, &first);
                None
            }
            Some(Mode::Session) => self.open(stream, peer, &first, deadline),
        };
        self.connecting.fetch_sub(1, Ordering::SeqCst);
        if let Some((session, stream, reader, receive)) = opened {
            let why = self.serve(&session, stream, reader, receive);
            self.ended(&session, why);
        }
        self.left();
    }

    // Pairing.

    fn pair(&self, mut stream: TcpStream, peer: SocketAddr, first: &[u8]) {
        let claimed = lock(&self.window).as_mut().and_then(|open| open.window.claim(Instant::now()).map(|key| (key, open.serial)));
        let Some((key, serial)) = claimed else {
            self.failed(peer.ip());
            return;
        };
        let outcome = self.pair_with(&mut stream, &key, first);
        {
            let mut window = lock(&self.window);
            if window.as_ref().is_some_and(|open| open.serial == serial) {
                match outcome {
                    Ok(()) => *window = None,
                    Err(_) => {
                        let open = window.as_mut().expect("checked");
                        open.window.failed();
                        if !open.window.alive(Instant::now()) {
                            *window = None;
                        }
                    }
                }
            }
        }
        match outcome {
            Ok(()) => {}
            Err(PairError::Handshake) => {
                self.failed(peer.ip());
                log::info!(target: "lumaremote", "pairing: a wrong code from {peer}");
            }
            Err(PairError::Refused(why)) => log::info!(target: "lumaremote", "pairing: refused {peer}: {why}"),
        }
        self.host.changed();
    }

    fn pair_with(&self, stream: &mut TcpStream, key: &[u8; 32], first: &[u8]) -> Result<(), PairError> {
        let refused = |why: &dyn std::fmt::Display| PairError::Refused(why.to_string());
        let (reply, Transport { mut send, mut receive }) =
            Responder::respond_with(&first[..PREAMBLE_LEN], key, self.secret(), &first[PREAMBLE_LEN..]).map_err(|_| PairError::Handshake)?;
        stream.write_all(&reply).map_err(|e| refused(&e))?;
        stream.set_read_timeout(Some(PAIR_FRAME_WITHIN)).map_err(|e| refused(&e))?;
        stream.set_write_timeout(Some(WRITE_WITHIN)).map_err(|e| refused(&e))?;
        let frame = wire::Reader::default().next(stream, &mut receive).map_err(|e| refused(&e))?;
        let mut error = || {
            let _ = wire::write(stream, &mut send, json!({"t": "error", "code": Fault::BadMessage.code()}).to_string().as_bytes());
        };
        let Some((name, model, app)) = pair_request(&frame) else {
            error();
            return Err(refused(&"not a pair message"));
        };
        let mut id = [0u8; 8];
        self.random.fill(&mut id).map_err(|_| refused(&"no random"))?;
        let device = Device { id: hex(&id), name, model, app, key: self.secret(), paired: unix_ms(SystemTime::now()), last: 0 };
        if let Err(e) = lock(&self.store).add(device.clone()) {
            error();
            return Err(refused(&e));
        }
        let paired = json!({
            "t": "paired", "device": device.id, "key": base64::encode(&device.key),
            "tv": {"id": hex(&self.tv), "name": self.name, "mac": self.mac.map(link::mac_text)},
        });
        if let Err(e) = wire::write(stream, &mut send, paired.to_string().as_bytes()) {
            // The phone never got its key: the row would only be clutter.
            let _ = lock(&self.store).remove(&device.id);
            return Err(refused(&e));
        }
        log::info!(target: "lumaremote", "paired {} ({}, {})", device.name, device.model, device.id);
        Ok(())
    }

    // Sessions.

    /// The handshake with each paired phone's key, then `hello`; the session is registered and welcomed.
    fn open(&self, mut stream: TcpStream, peer: SocketAddr, first: &[u8], deadline: Instant) -> Option<Opened> {
        let ephemeral = self.secret();
        let found = lock(&self.store).devices().iter().find_map(|d| {
            let answer = Responder::respond_with(&first[..PREAMBLE_LEN], &d.key, ephemeral, &first[PREAMBLE_LEN..]).ok()?;
            Some((d.id.clone(), d.name.clone(), answer))
        });
        let Some((device, stored_name, (reply, Transport { mut send, mut receive }))) = found else {
            self.failed(peer.ip());
            log::info!(target: "lumaremote", "no paired phone's key opens {peer}'s handshake");
            return None;
        };
        stream.write_all(&reply).ok()?;
        let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())?;
        stream.set_read_timeout(Some(left)).ok()?;
        let mut reader = wire::Reader::default();
        let frame = reader.next(&mut stream, &mut receive).ok()?;
        let Ok(Request::Hello { name, model, app, .. }) = proto::parse(&frame) else {
            let refusal = Refusal { fault: Fault::BadMessage, about: "hello".into(), id: None };
            let _ = wire::write(&mut stream, &mut send, refusal.reply().to_string().as_bytes());
            return None;
        };
        stream.set_read_timeout(None).ok()?;
        stream.set_write_timeout(Some(WRITE_WITHIN)).ok()?;
        let udp_key = self.secret();
        let state = self.read_state();
        let now = Instant::now();
        let shut = stream.try_clone().ok()?;
        let out = Out { stream: stream.try_clone().ok()?, send, broken: false };
        let replaced: Vec<Arc<Session>>;
        let session;
        // The writer is held from registering to `welcome`, so no `state` overtakes it.
        let mut writer;
        {
            let mut sessions = lock(&self.sessions);
            if self.stop.load(Ordering::SeqCst) {
                return None;
            }
            // The same phone again (another network, a reconnect) takes over from its old session.
            replaced = sessions.iter().filter(|s| s.device == device).cloned().collect();
            sessions.retain(|s| s.device != device);
            if sessions.len() >= MAX_SESSIONS {
                log::info!(target: "lumaremote", "{name} ({device}): {MAX_SESSIONS} sessions are live already");
                return None;
            }
            let mut udp_id = 0;
            while udp_id == 0 || sessions.iter().any(|s| s.udp_id == udp_id) {
                let mut b = [0u8; 4];
                self.random.fill(&mut b).ok()?;
                udp_id = u32::from_le_bytes(b);
            }
            session = Arc::new(Session {
                udp_id,
                udp_key: datagram::Key::new(&udp_key),
                device: device.clone(),
                name: if name.is_empty() { stored_name } else { name.clone() },
                stream: shut,
                out: Mutex::new(out),
                live: Mutex::new(Live {
                    heard: now,
                    held: Vec::new(),
                    pointer: Accumulator::default(),
                    seq_in: 0,
                    seq_out: 0,
                    frames: Meter::new(now),
                    datagrams: Meter::new(now),
                    sent: state.clone(),
                    ended: false,
                }),
            });
            writer = lock(&session.out);
            sessions.push(session.clone());
        }
        for old in replaced {
            old.end();
        }
        let welcome = json!({
            "t": "welcome", "v": crate::VERSION, "device": device,
            "tv": {"id": hex(&self.tv), "name": self.name, "mac": self.mac.map(link::mac_text), "accent": self.host.accent()},
            "udp": {"session": session.udp_id, "key": base64::encode(&udp_key)},
            "state": state,
        });
        let welcomed = write_out(&mut writer, &welcome);
        drop(writer);
        if !welcomed {
            lock(&self.sessions).retain(|s| !Arc::ptr_eq(s, &session));
            return None;
        }
        let _ = lock(&self.store).seen(&device, &name, &model, &app, unix_ms(SystemTime::now()));
        log::info!(target: "lumaremote", "{} ({device}) connected from {peer}", session.name);
        self.host.changed();
        Some((session, stream, reader, receive))
    }

    /// Reads frames until the phone leaves, the connection breaks or the session is ended; returns why.
    fn serve(&self, session: &Session, mut stream: TcpStream, mut reader: wire::Reader, mut receive: CipherState) -> &'static str {
        loop {
            let frame = match reader.next(&mut stream, &mut receive) {
                Ok(frame) => frame,
                Err(e) if e.kind() == io::ErrorKind::InvalidData => return "a frame failed",
                Err(_) => return "closed",
            };
            let now = Instant::now();
            let within = {
                let mut live = lock(&session.live);
                live.heard = now;
                live.frames.allow(now, FRAMES_PER_SECOND)
            };
            let reply = match proto::parse(&frame) {
                Ok(Request::Bye) => return "bye",
                Ok(request) if !within => Some(refuse(Fault::Rate, &request)),
                Ok(request) => self.request(session, request),
                Err(refusal) => Some(refusal.reply()),
            };
            if let Some(reply) = reply {
                if !session.send(&reply) {
                    return "a write failed";
                }
            }
        }
    }

    fn request(&self, session: &Session, request: Request) -> Option<Value> {
        let standby = self.standby.load(Ordering::SeqCst);
        let failed = |result: Result<(), Fault>| result.err().map(|fault| refuse(fault, &request));
        match &request {
            Request::Ping(n) => Some(json!({"t": "pong", "n": n})),
            Request::Hello { .. } | Request::Bye => Some(refuse(Fault::BadMessage, &request)),
            Request::Key { code, press, .. } if !standby || *code == proto::POWER => failed(self.key(session, *code, *press)),
            _ if standby => Some(refuse(Fault::Standby, &request)),
            Request::Text(text) => failed(self.host.text(text)),
            Request::Edit(edit) => failed(self.host.edit(*edit)),
            Request::Act { id, act } => Some(match self.host.act(act) {
                Ok(()) => json!({"t": "done", "id": id}),
                Err(fault) => json!({"t": "done", "id": id, "error": fault.code()}),
            }),
            Request::Get { id, what } => match self.host.list(*what) {
                Ok(items) => {
                    let pages = pages(items);
                    let last = pages.len() - 1;
                    for (i, items) in pages.into_iter().enumerate() {
                        if !session.send(&json!({"t": "list", "id": id, "what": what.name(), "items": items, "more": i < last})) {
                            break;
                        }
                    }
                    None
                }
                Err(fault) => Some(refuse(fault, &request)),
            },
            Request::Key { .. } => unreachable!("a key is taken above unless in standby"),
        }
    }

    /// A key, kept in the session's held keys while it is down, so a session that ends lets go of it.
    fn key(&self, session: &Session, code: u32, press: Press) -> Result<(), Fault> {
        let now = Instant::now();
        let mut live = lock(&session.live);
        let held = live.held.iter().position(|(c, _)| *c == code);
        match (press, held) {
            (Press::Tap, _) => self.host.key(code, None),
            (Press::Hold, Some(_)) => Ok(()),
            (Press::Down, Some(at)) => {
                live.held[at].1 = now + HELD_KEY;
                Ok(())
            }
            (Press::Hold | Press::Down, None) => {
                self.host.key(code, Some(true))?;
                let hold = match press {
                    Press::Down => HELD_KEY,
                    _ if code == MUTE => HOLD_MUTE,
                    _ => HOLD,
                };
                live.held.push((code, now + hold));
                Ok(())
            }
            (Press::Up, Some(at)) => {
                live.held.remove(at);
                self.host.key(code, Some(false))
            }
            (Press::Up, None) => Ok(()),
        }
    }

    /// The session is over: it leaves the list, and what its phone held is let go.
    fn ended(&self, session: &Arc<Session>, why: &str) {
        lock(&self.sessions).retain(|s| !Arc::ptr_eq(s, session));
        session.end();
        {
            let mut live = lock(&session.live);
            live.ended = true;
            for (code, _) in std::mem::take(&mut live.held) {
                let _ = self.host.key(code, Some(false));
            }
            let mut events = Vec::new();
            live.pointer.end(&mut events);
            if !events.is_empty() {
                self.host.pointer(&events, false);
            }
        }
        log::info!(target: "lumaremote", "{} ({}) left: {why}", session.name, session.device);
        self.host.changed();
    }

    // Datagrams and time.

    fn datagrams(self: Arc<Self>) {
        let mut buf = [0u8; 256];
        let mut events = Vec::new();
        let mut ticked = Instant::now();
        while !self.stop.load(Ordering::SeqCst) {
            if let Ok((n, from)) = self.udp.recv_from(&mut buf) {
                self.datagram(&buf[..n], from, Instant::now(), &mut events);
            }
            if ticked.elapsed() >= TICK {
                ticked = Instant::now();
                self.tick(ticked);
            }
        }
    }

    fn datagram(&self, packet: &[u8], from: SocketAddr, received: Instant, events: &mut Vec<Event>) {
        if !allowed(from.ip()) {
            return;
        }
        let Some(header) = Header::parse(packet) else { return };
        let Some(session) = lock(&self.sessions).iter().find(|s| s.udp_id == header.session).cloned() else { return };
        let Some(plain) = session.udp_key.open(datagram::TO_TV, packet) else { return };
        let mut live = lock(&session.live);
        if live.ended || header.seq <= live.seq_in {
            return;
        }
        live.seq_in = header.seq;
        live.heard = received;
        if !live.datagrams.allow(received, DATAGRAMS_PER_SECOND) {
            return;
        }
        match header.kind {
            datagram::POINTER => {
                let Some(pointer) = Pointer::parse(&plain) else { return };
                events.clear();
                // Totals are taken in standby too, so waking does not replay what moved meanwhile.
                live.pointer.apply(&pointer, events);
                let engaged = live.pointer.engaged();
                if !self.standby.load(Ordering::SeqCst) && (engaged || !events.is_empty()) {
                    self.host.pointer(events, engaged);
                }
            }
            datagram::PING => {
                let Some((id, time)) = datagram::parse_ping(&plain) else { return };
                live.seq_out += 1;
                let header = Header { kind: datagram::PONG, session: session.udp_id, seq: live.seq_out };
                let hold = u32::try_from(received.elapsed().as_micros()).unwrap_or(u32::MAX);
                let _ = self.udp.send_to(&session.udp_key.seal(datagram::TO_PHONE, header, &datagram::pong(id, time, hold)), from);
            }
            _ => {}
        }
    }

    /// Every [`TICK`]: an expired window closes; a silent session ends; held keys whose time is up are let go; phones get what changed.
    fn tick(&self, now: Instant) {
        let closed = {
            let mut window = lock(&self.window);
            let expired = window.as_ref().is_some_and(|open| !open.window.alive(now));
            if expired {
                *window = None;
            }
            expired
        };
        if closed {
            self.host.changed();
        }
        let sessions: Vec<Arc<Session>> = lock(&self.sessions).clone();
        if sessions.is_empty() {
            return;
        }
        let state = self.read_state();
        for session in sessions {
            let mut live = lock(&session.live);
            if live.ended {
                continue;
            }
            if now.saturating_duration_since(live.heard) >= SILENCE {
                drop(live);
                log::info!(target: "lumaremote", "{} ({}) went silent", session.name, session.device);
                session.end();
                continue;
            }
            let mut i = 0;
            while i < live.held.len() {
                if live.held[i].1 <= now {
                    let (code, _) = live.held.remove(i);
                    let _ = self.host.key(code, Some(false));
                } else {
                    i += 1;
                }
            }
            let mut changed: Map<String, Value> = state.iter().filter(|(k, v)| live.sent.get(*k) != Some(*v)).map(|(k, v)| (k.clone(), v.clone())).collect();
            if changed.is_empty() {
                continue;
            }
            live.sent = state.clone();
            drop(live);
            changed.insert("t".into(), "state".into());
            session.send(&Value::Object(changed));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{self, Session as Phone};
    use crate::datagram::Button;

    /// Records what the server asked of the TV: one line per key, text, edit and pointer call, and a count of `changed`.
    #[derive(Default)]
    struct Tv {
        calls: Mutex<(Vec<String>, usize)>,
        called: Condvar,
        state: Mutex<Map<String, Value>>,
    }

    impl Tv {
        fn record(&self, call: Option<String>) {
            let mut calls = lock(&self.calls);
            match call {
                Some(call) => calls.0.push(call),
                None => calls.1 += 1,
            }
            self.called.notify_all();
        }

        /// Waits for a call that starts with `prefix`, and takes the calls up to it.
        fn wait(&self, prefix: &str) -> Vec<String> {
            let calls = lock(&self.calls);
            let found = |c: &(Vec<String>, usize)| c.0.iter().any(|l| l.starts_with(prefix));
            let (mut calls, _) = self.called.wait_timeout_while(calls, Duration::from_secs(5), |c| !found(c)).unwrap();
            let at = calls.0.iter().position(|l| l.starts_with(prefix)).unwrap_or_else(|| panic!("no {prefix} in {:?}", calls.0));
            calls.0.drain(..=at).collect()
        }

        fn changes(&self) -> usize {
            lock(&self.calls).1
        }

        fn wait_change(&self, after: usize) {
            let calls = lock(&self.calls);
            let (calls, _) = self.called.wait_timeout_while(calls, Duration::from_secs(5), |c| c.1 <= after).unwrap();
            assert!(calls.1 > after, "no change");
        }

        fn set(&self, key: &str, value: Value) {
            lock(&self.state).insert(key.into(), value);
        }
    }

    impl Host for Tv {
        fn key(&self, code: u32, down: Option<bool>) -> Result<(), Fault> {
            self.record(Some(format!("key {code} {down:?}")));
            Ok(())
        }

        fn text(&self, text: &str) -> Result<(), Fault> {
            if text == "nowhere" {
                return Err(Fault::NoField);
            }
            self.record(Some(format!("text {text}")));
            Ok(())
        }

        fn edit(&self, edit: Edit) -> Result<(), Fault> {
            self.record(Some(format!("edit {edit:?}")));
            Ok(())
        }

        fn pointer(&self, events: &[Event], engaged: bool) {
            self.record(Some(format!("pointer {events:?} {engaged}")));
        }

        fn act(&self, act: &Act) -> Result<(), Fault> {
            match act {
                Act::Home => Ok(()),
                _ => Err(Fault::BadAction),
            }
        }

        fn list(&self, what: List) -> Result<Vec<Value>, Fault> {
            match what {
                List::Apps => Ok((0..400).map(|i| json!({"id": format!("com.example.app{i}"), "title": format!("App number {i}")})).collect()),
                _ => Err(Fault::Unavailable),
            }
        }

        fn state(&self) -> Map<String, Value> {
            lock(&self.state).clone()
        }

        fn accent(&self) -> String {
            "#5b8cff".into()
        }

        fn changed(&self) {
            self.record(None);
        }
    }

    fn start() -> (Server, Arc<Tv>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let tv = Arc::new(Tv::default());
        tv.set("power", json!("on"));
        tv.set("fg", json!(""));
        let config = Config {
            bind: Ipv4Addr::LOCALHOST,
            port: 0,
            name: "LG C4".into(),
            mac: Some([0x74, 0xc1, 0x7e, 0xbd, 0xae, 0x30]),
            store: dir.path().join("devices.json"),
        };
        let server = Server::start(config, tv.clone()).unwrap();
        (server, tv, dir)
    }

    /// Pairs a phone through a new window; returns its key once the server is done with the window.
    fn paired(server: &Server, tv: &Tv) -> [u8; 32] {
        let code = PairLink::parse(server.open_pairing().unwrap()["qr"].as_str().unwrap()).unwrap().code;
        let before = tv.changes();
        let phone = client::pair(server.local_addr(), &pairing::key(&code, &server.tv()), "Test phone", "iPhone17,1", "test").unwrap();
        tv.wait_change(before);
        phone.key
    }

    fn open(server: &Server, tv: &Tv, key: &[u8; 32]) -> Phone {
        let before = tv.changes();
        let phone = Phone::open(server.local_addr(), key, &json!({"t": "hello", "name": "Test phone", "model": "iPhone17,1", "app": "test"})).unwrap();
        tv.wait_change(before);
        phone
    }

    fn unanswered<T>(result: io::Result<T>) -> bool {
        matches!(result, Err(e) if e.kind() == io::ErrorKind::UnexpectedEof)
    }

    /// Sends `msg` and a ping, and returns what came back before the pong.
    fn ask(phone: &mut Phone, msg: Value) -> Vec<Value> {
        phone.send(&msg).unwrap();
        phone.send(&json!({"t": "ping", "n": 1})).unwrap();
        let mut got = Vec::new();
        loop {
            let reply = phone.receive().unwrap();
            if reply["t"] == "pong" {
                return got;
            }
            got.push(reply);
        }
    }

    #[test]
    fn a_window_pairs_one_phone_and_wrong_keys_get_no_answer() {
        let (server, tv, _dir) = start();
        let addr = server.local_addr();
        assert!(unanswered(client::pair(addr, &[7; 32], "x", "", "")), "no window");
        let pairing = server.open_pairing().unwrap();
        assert_eq!(pairing["code"].as_str().unwrap().len(), 9);
        assert_eq!(server.txt().last().unwrap(), &("pair".to_string(), "1".to_string()));
        for _ in 1..pairing::TRIES {
            assert!(unanswered(client::pair(addr, &[7; 32], "x", "", "")));
        }
        assert!(!server.status()["pairing"].is_null(), "four wrong codes leave the window open");
        assert!(unanswered(client::pair(addr, &[7; 32], "x", "", "")));
        assert!(server.status()["pairing"].is_null(), "the fifth closes it");

        let key = paired(&server, &tv);
        let status = server.status();
        assert!(status["pairing"].is_null(), "a window pairs once");
        assert_eq!(status["devices"][0]["name"], "Test phone");
        assert_eq!(status["devices"][0]["live"], false);
        assert!(unanswered(Phone::open(addr, &[9; 32], &json!({"t": "hello"}))), "another key opens no session");
        let phone = open(&server, &tv, &key);
        assert_eq!(phone.welcome["tv"]["mac"], "74:c1:7e:bd:ae:30");
        assert_eq!(phone.welcome["state"]["power"], "on");
        assert_eq!(server.status()["devices"][0]["live"], true);
    }

    #[test]
    fn a_session_maps_keys_text_actions_and_lists() {
        let (server, tv, _dir) = start();
        let key = paired(&server, &tv);
        let mut phone = open(&server, &tv, &key);
        assert!(ask(&mut phone, json!({"t": "key", "k": "ok", "a": "tap"})).is_empty());
        assert_eq!(tv.wait("key"), ["key 28 None"]);
        ask(&mut phone, json!({"t": "key", "k": "up", "a": "down"}));
        ask(&mut phone, json!({"t": "key", "k": "up", "a": "up"}));
        assert_eq!(tv.wait("key 103 Some(false)"), ["key 103 Some(true)", "key 103 Some(false)"]);
        assert_eq!(ask(&mut phone, json!({"t": "key", "k": "pageup", "a": "tap"}))[0]["code"], "bad-key");
        ask(&mut phone, json!({"t": "text", "s": "Hello"}));
        assert_eq!(tv.wait("text"), ["text Hello"]);
        assert_eq!(ask(&mut phone, json!({"t": "text", "s": "nowhere"})), [json!({"t": "error", "code": "no-field", "about": "text"})]);
        ask(&mut phone, json!({"t": "edit", "op": "backspace", "n": 3}));
        assert_eq!(tv.wait("edit"), ["edit Backspace(3)"]);
        assert_eq!(ask(&mut phone, json!({"t": "act", "id": 4, "do": "home"})), [json!({"t": "done", "id": 4})]);
        assert_eq!(ask(&mut phone, json!({"t": "act", "id": 5, "do": "guide"})), [json!({"t": "done", "id": 5, "error": "bad-action"})]);
        let pages = ask(&mut phone, json!({"t": "get", "id": 6, "what": "apps"}));
        assert!(pages.len() > 1);
        assert!(pages.iter().all(|p| p["id"] == 6 && p["what"] == "apps"));
        assert_eq!(pages.iter().map(|p| p["items"].as_array().unwrap().len()).sum::<usize>(), 400);
        assert_eq!(pages.iter().position(|p| p["more"] == false), Some(pages.len() - 1));

        tv.set("power", json!("standby"));
        server.inner.tick(Instant::now());
        assert_eq!(phone.receive().unwrap(), json!({"t": "state", "power": "standby"}));
        assert_eq!(ask(&mut phone, json!({"t": "key", "k": "ok", "a": "tap"}))[0]["code"], "standby");
        ask(&mut phone, json!({"t": "key", "k": "power", "a": "tap"}));
        assert_eq!(tv.wait("key"), ["key 116 None"]);
    }

    #[test]
    fn pointer_datagrams_apply_once_in_order_and_pings_are_answered() {
        let (server, tv, _dir) = start();
        let key = paired(&server, &tv);
        let mut phone = open(&server, &tv, &key);
        let mut p = Pointer { x: 32, y: -16, flags: datagram::ENGAGED, ..Pointer::default() };
        phone.pointer(&p).unwrap();
        assert_eq!(tv.wait("pointer"), ["pointer [Engaged(true), Move { dx: 32, dy: -16 }] true"]);
        // A replay and a forgery are dropped.
        phone.resend().unwrap();
        phone.send_raw(&[0x4c, 0x52, 1, 1, 0, 0, 0, 0, 9, 0, 0, 0, 1, 2, 3]).unwrap();
        p.left = 1;
        p.flags = 0;
        phone.pointer(&p).unwrap();
        assert_eq!(tv.wait("pointer"), ["pointer [Click(Left), Engaged(false)] false"]);
        let (id, time, _) = phone.ping(7, 123_456).unwrap();
        assert_eq!((id, time), (7, 123_456));
        p.buttons = datagram::LEFT;
        p.left = 2;
        phone.pointer(&p).unwrap();
        assert_eq!(tv.wait("pointer"), ["pointer [Press(Left)] false"]);
        // The session ending lets go of the held button.
        drop(phone);
        assert_eq!(tv.wait("pointer"), [format!("pointer {:?} false", [Event::Release(Button::Left)])]);
    }

    #[test]
    fn removed_and_silent_phones_lose_their_session_and_held_keys() {
        let (server, tv, _dir) = start();
        let key = paired(&server, &tv);
        let mut phone = open(&server, &tv, &key);
        ask(&mut phone, json!({"t": "key", "k": "volup", "a": "hold"}));
        assert_eq!(tv.wait("key"), ["key 115 Some(true)"]);
        server.inner.tick(Instant::now() + HOLD);
        assert_eq!(tv.wait("key"), ["key 115 Some(false)"]);
        ask(&mut phone, json!({"t": "key", "k": "down", "a": "down"}));
        assert_eq!(tv.wait("key"), ["key 108 Some(true)"]);
        server.inner.tick(Instant::now() + SILENCE);
        assert_eq!(tv.wait("key"), ["key 108 Some(false)"]);
        assert!(phone.receive().is_err());

        let mut phone = open(&server, &tv, &key);
        let id = server.status()["devices"][0]["id"].as_str().unwrap().to_string();
        assert!(server.forget(&id).unwrap());
        assert_eq!(phone.receive().unwrap(), json!({"t": "revoked"}));
        assert!(phone.receive().is_err());
        assert!(unanswered(Phone::open(server.local_addr(), &key, &json!({"t": "hello"}))));
    }

    #[test]
    fn pages_split_long_lists_and_send_an_empty_one() {
        assert_eq!(pages(Vec::new()), vec![Vec::<Value>::new()]);
        let split = pages((0..300).map(|i| json!({"id": "x".repeat(40), "n": i})).collect());
        assert!(split.len() > 1);
        assert!(split.iter().all(|p| Value::from(p.clone()).to_string().len() <= PAGE_BYTES + 2));
        assert_eq!(split.iter().map(Vec::len).sum::<usize>(), 300);
    }
}
