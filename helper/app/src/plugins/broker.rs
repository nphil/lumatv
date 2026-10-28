//! The broker: the unix socket plugins connect to, and one reader thread and one writer thread per connection.
//!
//! **It exists only while a plugin is switched on.** `Plugins` (the manager) starts it when the roster has a member and ends it
//! when none is: with every plugin off there is no socket and no thread of the broker.
//!
//! **A connection is a plugin, and nothing else.** The user id at the other end (`SO_PEERCRED`) is looked up in the roster. A user
//! that is not on it makes the file be read again (a plugin switched on a moment ago connects before the next scan; at most once a
//! second), and one that is still not is told so (`not_a_plugin`, which the SDK takes as "not yet") and dropped. A second connection
//! of a plugin replaces the first.
//!
//! **Everything that arrives and everything that is sent is bounded.** A frame is at most 64 KiB (checked by the framing before a
//! byte of it is kept), a connection that has not said `hello` in the handshake time is dropped, a plugin may send 100 messages a
//! second (200 as a burst) and one that goes on past it is hung up on, descriptors that arrive are closed (this TV takes none), and
//! what is queued for a plugin that does not read is capped at 256 frames and 1 MiB, at which the connection ends.

use super::roster::Roster;
use super::session::{Outlet, Reply, Session, Shared, wire_body};
use parking_lot::Mutex;
use plugin_api::limits::{MESSAGES_PER_S, OUTBOX_BYTES, OUTBOX_FRAMES, RATE_STRIKES};
use plugin_api::transport::{Inbox, Read};
use plugin_api::wire::{Decoded, ToBase, ToPlugin};
use std::collections::BTreeMap;
use std::io::{self, Read as _, Write as _};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Connections held at once, plugins and the not-yet-identified together.
const MAX_CONNECTIONS: usize = 2 * plugin_api::MAX_PLUGINS as usize;
/// How long the writer waits for a plugin that does not read before it gives up on the connection.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest a reader sleeps, so that a stop or a close is noticed even with nothing due.
const LONGEST_SLEEP: Duration = Duration::from_secs(30);
const RELOAD_EVERY_MS: u64 = 1000;
const STACK_BYTES: usize = 512 * 1024;

/// How long the broker waits for a plugin's questions and handshake (changed in tests).
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub handshake: Duration,
    /// The user id of plugin 0 (see `Roster::load`), for the roster the broker reads again when an unknown user connects.
    pub first_uid: u32,
}

/// A connection as the rest of the helper sees it: where to speak to the plugin, and how to end it.
pub struct ConnCtl {
    stream: UnixStream,
    /// One byte written here wakes the reader (it polls the other end): something it must act on has been queued for it.
    wake_tx: UnixStream,
    tx: Mutex<Option<SyncSender<Vec<u8>>>>,
    queued: AtomicUsize,
    closed: AtomicBool,
    shared: Arc<Shared>,
    plugin: Mutex<Option<String>>,
}

impl ConnCtl {
    /// Ends the connection now: whatever is queued is lost.
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }

    /// Stops accepting messages for the writer; what is queued is still written.
    fn finish(&self) {
        self.tx.lock().take();
    }
}

impl Outlet for ConnCtl {
    fn poke(&self) {
        // A full pipe means the reader is already woken.
        let _ = (&self.wake_tx).write(&[1]);
    }

    fn send(&self, message: ToPlugin) -> bool {
        if self.closed.load(Ordering::SeqCst) {
            return false;
        }
        let device = matches!(message, ToPlugin::Welcome(_))
            .then(|| self.shared.device_facts())
            .flatten();
        let body = wire_body(&message, device);
        let Ok(frame) = plugin_api::frame::encode(&body) else {
            return false;
        };
        if self.queued.load(Ordering::SeqCst) + frame.len() > OUTBOX_BYTES {
            self.close();
            return false;
        }
        let guard = self.tx.lock();
        let Some(tx) = guard.as_ref() else {
            return false;
        };
        let length = frame.len();
        // Counted before the writer can take the frame: counted after, the writer's subtraction can come first and wrap the count.
        self.queued.fetch_add(length, Ordering::SeqCst);
        match tx.try_send(frame) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.queued.fetch_sub(length, Ordering::SeqCst);
                drop(guard);
                self.close();
                false
            }
            Err(TrySendError::Disconnected(_)) => {
                self.queued.fetch_sub(length, Ordering::SeqCst);
                false
            }
        }
    }
}

/// A connection in the table: how to reach it, and its thread (taken when it is joined).
type Conn = (Arc<ConnCtl>, Option<JoinHandle<()>>);

struct Inner {
    shared: Arc<Shared>,
    roster: Mutex<Roster>,
    dir: PathBuf,
    socket: PathBuf,
    timing: Timing,
    stopping: AtomicBool,
    next_conn: AtomicU64,
    last_reload: AtomicU64,
    conns: Mutex<BTreeMap<u64, Conn>>,
}

pub struct Broker {
    inner: Arc<Inner>,
    accept: Option<JoinHandle<()>>,
}

fn private_dir(path: &Path) -> io::Result<()> {
    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o750))?;
    Ok(())
}

/// Hands the directory and the socket to the plugins' group when the helper may (it runs as root on the TV; a test does not).
/// The roster's user check is what decides who is served: this only decides who can reach the socket at all.
fn share_with_group(path: &Path) {
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
        return;
    };
    // SAFETY: a valid NUL-terminated path; (uid_t)-1 leaves the owner as it is.
    let rc = unsafe { libc::chown(c.as_ptr(), u32::MAX, plugin_api::PLUGINS_GID) };
    if rc != 0 {
        crate::log_debug!(
            "plugins: {} stays in its own group ({})",
            path.display(),
            io::Error::last_os_error()
        );
    }
}

impl Broker {
    pub fn start(
        shared: Arc<Shared>,
        roster: Roster,
        dir: PathBuf,
        socket: PathBuf,
        timing: Timing,
    ) -> io::Result<Self> {
        if let Some(parent) = socket.parent() {
            private_dir(parent)?;
            share_with_group(parent);
        }
        match std::fs::symlink_metadata(&socket) {
            Ok(meta) if std::os::unix::fs::FileTypeExt::is_socket(&meta.file_type()) => {
                std::fs::remove_file(&socket)?;
            }
            Ok(_) => {
                return Err(io::Error::other(format!(
                    "{} exists and is not a socket",
                    socket.display()
                )));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o660))?;
        share_with_group(&socket);
        let inner = Arc::new(Inner {
            shared,
            roster: Mutex::new(roster),
            dir,
            socket,
            timing,
            stopping: AtomicBool::new(false),
            next_conn: AtomicU64::new(1),
            last_reload: AtomicU64::new(0),
            conns: Mutex::new(BTreeMap::new()),
        });
        let accept = {
            let inner = inner.clone();
            thread::Builder::new()
                .name("plugins-accept".into())
                .stack_size(STACK_BYTES)
                .spawn(move || accept_loop(&inner, &listener))?
        };
        Ok(Self {
            inner,
            accept: Some(accept),
        })
    }

    /// A newer roster: plugins that are no longer on are asked to finish and then cut off.
    pub fn set_roster(&self, roster: Roster) {
        let gone: Vec<Arc<ConnCtl>> = self
            .inner
            .conns
            .lock()
            .values()
            .map(|(ctl, _)| ctl.clone())
            .filter(|ctl| {
                ctl.plugin.lock().as_deref().is_some_and(|id| {
                    !roster
                        .members
                        .iter()
                        .any(|m| m.id == id && Some(m.uid) == peer_uid(&ctl.stream))
                })
            })
            .collect();
        *self.inner.roster.lock() = roster;
        if !gone.is_empty() {
            for ctl in &gone {
                ctl.send(ToPlugin::Shutdown {
                    reason: "disabled".into(),
                    grace_ms: 300,
                });
            }
            thread::sleep(Duration::from_millis(300));
            for ctl in &gone {
                ctl.close();
            }
        }
    }

    /// Plugins with an open connection (identified or not).
    pub fn connections(&self) -> usize {
        self.inner.conns.lock().len()
    }

    /// Asks every plugin to finish (`reason` is what it is told: "stopping" or "disabled"), closes the socket and ends every thread.
    pub fn stop(mut self, reason: &str) {
        self.shutdown(reason);
    }

    fn shutdown(&mut self, reason: &str) {
        let inner = &self.inner;
        inner.stopping.store(true, Ordering::SeqCst);
        // Wakes the accept: it is blocked in accept(2).
        let _ = UnixStream::connect(&inner.socket);
        if let Some(handle) = self.accept.take() {
            let _ = handle.join();
        }
        let all: Vec<Conn> = std::mem::take(&mut *inner.conns.lock())
            .into_values()
            .collect();
        for (ctl, _) in &all {
            ctl.send(ToPlugin::Shutdown {
                reason: reason.to_owned(),
                grace_ms: 300,
            });
        }
        if !all.is_empty() {
            // The writer threads put Shutdown on the wire; closing before they have (a loaded machine can starve one for longer than a
            // fixed pause) would lose it. Bounded by the grace the plugin was told.
            let flushed = Instant::now() + Duration::from_millis(300);
            while Instant::now() < flushed
                && all.iter().any(|(ctl, _)| {
                    !ctl.closed.load(Ordering::SeqCst) && ctl.queued.load(Ordering::SeqCst) > 0
                })
            {
                thread::sleep(Duration::from_millis(2));
            }
            thread::sleep(Duration::from_millis(150));
        }
        for (ctl, _) in &all {
            ctl.close();
        }
        for (_, handle) in all {
            if let Some(handle) = handle {
                let _ = handle.join();
            }
        }
        let _ = std::fs::remove_file(&inner.socket);
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        if self.accept.is_some() {
            self.shutdown("stopping");
        }
    }
}

fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: SO_PEERCRED writes a `ucred` of at most `length` bytes into `cred`.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast(),
            &mut length,
        )
    };
    (rc == 0).then_some(cred.uid)
}

fn accept_loop(inner: &Arc<Inner>, listener: &UnixListener) {
    for stream in listener.incoming() {
        if inner.stopping.load(Ordering::SeqCst) {
            return;
        }
        let Ok(stream) = stream else {
            thread::sleep(Duration::from_millis(50));
            continue;
        };
        // Finished connections are dropped from the table here, so it holds only live ones.
        inner
            .conns
            .lock()
            .retain(|_, (_, handle)| handle.as_ref().is_some_and(|h| !h.is_finished()));
        if inner.conns.lock().len() >= MAX_CONNECTIONS {
            continue;
        }
        let Ok(copy) = stream.try_clone() else {
            continue;
        };
        if stream.set_nonblocking(true).is_err() {
            continue;
        }
        let Ok((wake_tx, wake_rx)) = UnixStream::pair() else {
            continue;
        };
        if wake_tx.set_nonblocking(true).is_err() || wake_rx.set_nonblocking(true).is_err() {
            continue;
        }
        let (tx, rx) = sync_channel::<Vec<u8>>(OUTBOX_FRAMES);
        let ctl = Arc::new(ConnCtl {
            shared: inner.shared.clone(),
            stream: copy,
            wake_tx,
            tx: Mutex::new(Some(tx)),
            queued: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            plugin: Mutex::new(None),
        });
        let conn = inner.next_conn.fetch_add(1, Ordering::Relaxed);
        let spawned = {
            let (inner, ctl) = (inner.clone(), ctl.clone());
            thread::Builder::new()
                .name("plugin-conn".into())
                .stack_size(STACK_BYTES)
                .spawn(move || serve(&inner, &ctl, stream, &wake_rx, rx, conn))
        };
        match spawned {
            Ok(handle) => {
                inner.conns.lock().insert(conn, (ctl, Some(handle)));
            }
            Err(e) => crate::log_warn!("plugins: no thread for a connection: {e}"),
        }
    }
}

/// Writes `bytes` to a non-blocking socket, waiting for room up to `timeout` in all.
fn write_all(fd: RawFd, mut bytes: &[u8], timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while !bytes.is_empty() {
        // SAFETY: writes from a live slice to a socket; MSG_NOSIGNAL turns a broken pipe into an error, not a signal.
        let n = unsafe { libc::send(fd, bytes.as_ptr().cast(), bytes.len(), libc::MSG_NOSIGNAL) };
        if n > 0 {
            bytes = &bytes[n as usize..];
            continue;
        }
        let e = io::Error::last_os_error();
        match e.kind() {
            io::ErrorKind::Interrupted => {}
            io::ErrorKind::WouldBlock => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() || !poll(fd, libc::POLLOUT, Some(left)) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

/// Waits until `fd` has `events` (or an error or hang-up, which the next read or write reports) or `timeout` runs out.
fn poll(fd: RawFd, events: libc::c_short, timeout: Option<Duration>) -> bool {
    let mut item = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    let ms = timeout.map_or(-1, |t| t.as_millis().min(i32::MAX as u128) as i32);
    // SAFETY: one valid pollfd.
    let rc = unsafe { libc::poll(&mut item, 1, ms) };
    rc > 0
}

/// Waits until either descriptor is readable (or has an error or hang-up, which the next read reports) or `timeout` runs out.
fn poll_two(a: RawFd, b: RawFd, timeout: Duration) {
    let mut items = [
        libc::pollfd {
            fd: a,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: b,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: two valid pollfds.
    unsafe { libc::poll(items.as_mut_ptr(), 2, ms) };
}

fn writer(ctl: &ConnCtl, fd: RawFd, rx: &Receiver<Vec<u8>>) {
    while let Ok(frame) = rx.recv() {
        if ctl.closed.load(Ordering::SeqCst) {
            break;
        }
        let length = frame.len();
        let ok = write_all(fd, &frame, WRITE_TIMEOUT);
        ctl.queued.fetch_sub(length, Ordering::SeqCst);
        if !ok {
            ctl.close();
            break;
        }
    }
}

struct Bucket {
    tokens: f64,
    refilled: Instant,
    strikes: u32,
}

impl Bucket {
    fn new() -> Self {
        Self {
            tokens: f64::from(2 * MESSAGES_PER_S),
            refilled: Instant::now(),
            strikes: 0,
        }
    }

    /// Whether one more message is taken. `Err(true)`: the plugin has gone on too long and is to be hung up on.
    fn take(&mut self) -> Result<(), bool> {
        let now = Instant::now();
        let rate = f64::from(MESSAGES_PER_S);
        self.tokens =
            (self.tokens + now.duration_since(self.refilled).as_secs_f64() * rate).min(2.0 * rate);
        self.refilled = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            self.strikes = 0;
            Ok(())
        } else {
            self.strikes += 1;
            Err(self.strikes >= RATE_STRIKES)
        }
    }
}

fn reload(inner: &Inner) {
    let now = inner.shared.now_ms();
    let last = inner.last_reload.load(Ordering::SeqCst);
    if now.saturating_sub(last) < RELOAD_EVERY_MS
        || inner
            .last_reload
            .compare_exchange(last, now, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    {
        return;
    }
    let (roster, problem) = Roster::load(&inner.dir, inner.timing.first_uid);
    inner.shared.set_roster(&roster, problem);
    *inner.roster.lock() = roster;
}

fn serve(
    inner: &Arc<Inner>,
    ctl: &Arc<ConnCtl>,
    stream: UnixStream,
    wake_rx: &UnixStream,
    rx: Receiver<Vec<u8>>,
    conn: u64,
) {
    let fd = stream.as_raw_fd();
    let writer_thread = {
        let ctl = ctl.clone();
        thread::Builder::new()
            .name("plugin-write".into())
            .stack_size(STACK_BYTES)
            .spawn(move || writer(&ctl, fd, &rx))
    };
    let Ok(writer_thread) = writer_thread else {
        ctl.close();
        return;
    };
    run(inner, ctl, &stream, wake_rx, conn);
    // What was queued (a refusal's reason) is written before the connection ends.
    ctl.finish();
    let _ = writer_thread.join();
    linger(&stream);
    ctl.close();
}

/// Ends our half of the stream and reads what the peer still sends until it closes (at most a moment): closing a socket that has
/// unread data in it resets the connection, and a reset discards the reason the plugin has not read yet.
fn linger(stream: &UnixStream) {
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let end = Instant::now() + Duration::from_millis(300);
    let mut scratch = [0u8; 4096];
    loop {
        match (&*stream).read(&mut scratch) {
            Ok(0) => return,
            Ok(_) => continue,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let left = end.saturating_duration_since(Instant::now());
                if left.is_zero() || !poll(stream.as_raw_fd(), libc::POLLIN, Some(left)) {
                    return;
                }
            }
            Err(_) => return,
        }
    }
}

fn fatal(ctl: &ConnCtl, code: &str, message: &str) {
    ctl.send(ToPlugin::Error {
        code: code.to_owned(),
        message: message.to_owned(),
        fatal: true,
    });
}

fn run(
    inner: &Arc<Inner>,
    ctl: &Arc<ConnCtl>,
    stream: &UnixStream,
    wake_rx: &UnixStream,
    conn: u64,
) {
    let fd = stream.as_raw_fd();
    let Some(uid) = peer_uid(stream) else {
        return;
    };
    let mut member = inner.roster.lock().by_uid(uid).cloned();
    if member.is_none() {
        reload(inner);
        member = inner.roster.lock().by_uid(uid).cloned();
    }
    let Some(member) = member else {
        fatal(
            ctl,
            plugin_api::NOT_A_PLUGIN,
            "no plugin of this user is switched on",
        );
        return;
    };
    let id = member.id.clone();
    *ctl.plugin.lock() = Some(id.clone());
    // The newest connection of a plugin is the live one: a plugin that was restarted does not wait for the dead one to be noticed.
    for (other_conn, (other, _)) in inner.conns.lock().iter() {
        if *other_conn != conn && other.plugin.lock().as_deref() == Some(id.as_str()) {
            other.close();
        }
    }
    crate::log_info!("plugins: {id} connected");
    let mut session = Session::new(member, inner.shared.clone(), conn, ctl.clone());
    let started = Instant::now();
    let mut said_hello = false;
    let mut inbox = Inbox::new();
    let mut bucket = Bucket::new();
    'connection: loop {
        // `stopping` is not tested here: the connection ends by `shutdown`, which sends Shutdown and then closes it. Ending on the flag
        // finishes the writer's queue before Shutdown is on it, and the plugin sees the stream close without being told why.
        if ctl.closed.load(Ordering::SeqCst) {
            break;
        }
        // Sleep until data, the handshake's end, or the session's next deadline.
        let mut wait = LONGEST_SLEEP;
        if !said_hello {
            wait = wait.min(inner.timing.handshake.saturating_sub(started.elapsed()));
        }
        if let Some(due) = session.next_deadline_ms() {
            let now = inner.shared.now_ms();
            wait = wait.min(Duration::from_millis(due.saturating_sub(now) + 1));
        }
        poll_two(fd, wake_rx.as_raw_fd(), wait);
        let mut drain = [0u8; 64];
        while matches!((&*wake_rx).read(&mut drain), Ok(n) if n > 0) {}
        loop {
            match inbox.read(fd) {
                Ok(Read::Data) => {}
                Ok(Read::WouldBlock) => break,
                Ok(Read::Eof) | Err(_) => break 'connection,
                Ok(Read::Fault(why)) => {
                    fatal(ctl, "bad_stream", &why.to_string());
                    break 'connection;
                }
            }
            loop {
                let frame = match inbox.next_frame() {
                    Ok(Some(frame)) => frame,
                    Ok(None) => break,
                    Err(why) => {
                        fatal(ctl, "bad_frame", &why.to_string());
                        break 'connection;
                    }
                };
                // This TV takes no descriptors: whatever came with the frame is closed.
                while inbox.take_fd().is_some() {}
                said_hello = true;
                if let Err(hang_up) = bucket.take() {
                    if hang_up {
                        fatal(ctl, "rate_exceeded", "too many messages: hanging up");
                        break 'connection;
                    }
                    ctl.send(ToPlugin::Refused {
                        what: "message".into(),
                        code: "rate".into(),
                        reason: format!("more than {MESSAGES_PER_S} messages a second"),
                        retry_after_ms: None,
                    });
                    continue;
                }
                match ToBase::decode(&frame) {
                    Ok(Decoded::Known(message)) => {
                        let Reply { messages, hangup } = session.message(message);
                        for message in messages {
                            ctl.send(message);
                        }
                        if hangup {
                            break 'connection;
                        }
                    }
                    Ok(Decoded::Unknown(t)) => {
                        ctl.send(ToPlugin::Error {
                            code: "unknown_type".into(),
                            message: format!("this level has no message type {t:?}"),
                            fatal: false,
                        });
                    }
                    Err(
                        e @ (plugin_api::wire::DecodeError::NotJson(_)
                        | plugin_api::wire::DecodeError::NotAnObject
                        | plugin_api::wire::DecodeError::NoType),
                    ) => {
                        fatal(ctl, "bad_frame", &e.to_string());
                        break 'connection;
                    }
                    Err(e) => {
                        ctl.send(ToPlugin::Error {
                            code: "bad_message".into(),
                            message: e.to_string(),
                            fatal: false,
                        });
                    }
                }
            }
        }
        session.tick();
        if !said_hello && started.elapsed() >= inner.timing.handshake {
            fatal(ctl, "handshake_timeout", "hello was not said in time");
            break;
        }
    }
    crate::log_info!("plugins: {id} disconnected");
    drop(session);
}
