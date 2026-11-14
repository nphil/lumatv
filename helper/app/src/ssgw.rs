//! The Home Assistant gateway: LumaTV's `ssgw` (`helper/ssgw`) serving port 3001, the second-screen (SSAP) websocket of Home
//! Assistant's `webostv` integration, in place of LG's gateway, as a plugin of the helper's own that asks first (`plugins/local.rs`'s
//! `OptIn`). The design is `docs/research/ss-gateway-rust-plan.md`.
//!
//! **On**, the plugin has `tools/ssgw.sh on` hand port 3001's connections to `ssgw-accept` and runs `ssgw` as a user of its own,
//! confined as a plugin's program is with room for [`FILES`] open files; as root when `luna-send` does not answer that user, or the
//! gateway exits saying it cannot work as it. `ssgw-accept` gives each connection to the gateway, or to LG's gateway whenever this one
//! is not there or does not take it in time, so falling back needs nothing from the helper. **Off**, the gateway is stopped and
//! `ssgw.sh off` gives the port back to LG's entry. The helper stopping stops the gateway and leaves the entry: connections go LG's way
//! until it is back.
//!
//! **A crash** (an exit the helper did not ask for, a start that failed, or 30 s without an answer on the control socket, after which
//! the gateway is killed) is kept under `crash/<time>-<n>/` with the gateway's last 300 log lines and the last URIs it served, shown on
//! the page until it is marked read, and told once to the notify service named on the page through the TV's Home Assistant link. The
//! gateway starts again after 2 s, doubling to 60 s. After three crashes within 10 min, or at once when it exits saying it cannot serve
//! this TV ([`UNUSABLE`]), the plugin stops trying until the next boot or Try again, and says so once.

use crate::config::Config;
use crate::ctx::Ctx;
use crate::ha_store;
use crate::plugins::firstparty::clip;
use crate::plugins::local::{Handler, Local, OptIn, Spec};
use crate::plugins::runner::{confine, confine_files, end, sweep};
use crate::plugins::session::{MAX_FACT_LINE, Shared};
use crate::timeutil::{now_ms, utc_stamp};
use crate::voice_ws::Ha;
use ha_link::AddressPolicy;
use parking_lot::Mutex;
use plugin_api::wire::ToPlugin;
use serde_json::{Map, Value as Json, json};
use settings_schema::{Value as Setting, Values};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The plugin's id, which is also its settings owner.
pub const ID: &str = "ssgw";
pub const OPT_IN: OptIn = OptIn {
    spec: Spec {
        id: ID,
        name: "Home Assistant gateway",
        version: "1",
        permissions: &["settings"],
    },
    consent: "Answers Home Assistant's LG webOS TV integration with LumaTV's own gateway in place of LG's. Home Assistant keeps its \
              pairing, LG's gateway takes over whenever LumaTV's is not running, and each crash is kept on the TV and told to the \
              phone named on this page through Home Assistant.",
};

const NOTIFY: &str = "notify.service";
const NOTIFY_DEFAULT: &str = "notify.mobile_app_nitins_iphone";
const ACK: &str = "ack";
const RETRY: &str = "retry";
const FORGET: &str = "forget";
/// The gateway's user and group: none of the TV's, below Beszel's agent (19999) and the plugins' users, which count up from 20000.
const GATEWAY_ID: u32 = 19_998;
/// Each Home Assistant session holds its connection and, while LG serves it, the relay to LG's gateway: a plugin's 64 is too few.
const FILES: u64 = 256;
/// LG's input sockets that a Home Assistant session's pointer drives. They are root's alone (`srwxrwx--- root:root`), so the helper
/// connects them for a gateway running as its own user and hands it the connection.
const NETINPUT: [&str; 3] = ["/tmp/netinput.pointer.sock", "/tmp/netinput.gamepad.sock", "/tmp/netinput.touchpanel.sock"];
/// How often the gateway is asked whether it answers, and how many missed answers in a row make it hung.
const TICK: Duration = Duration::from_secs(10);
const MISSES: u32 = 3;
/// How long the gateway has to close its sessions when it is stopped.
const GRACE: Duration = Duration::from_secs(3);
/// The pause before a gateway that crashed is started again: doubling from the first to the last, and back to the first after a run
/// of [`STEADY`].
const FIRST_PAUSE: Duration = Duration::from_secs(2);
const LAST_PAUSE: Duration = Duration::from_secs(60);
const STEADY: Duration = Duration::from_secs(600);
/// This many crashes within [`WINDOW`] and the plugin stops trying.
const CRASHES: usize = 3;
const WINDOW: Duration = Duration::from_secs(600);
/// The gateway's exit status for a TV it cannot serve: LG's files are not the ones it pins, or the TV's key is out of its reach.
const UNUSABLE: i32 = 78;
const LOG_LINES: usize = 300;
const LOG_LINE_BYTES: u64 = 2048;
/// Crash records kept.
const RECORDS: usize = 8;
/// How often the page's other lines (the entry, LG's service host's repair, LG's extras) are read again.
const SIDE_EVERY: Duration = Duration::from_secs(600);
/// The longest the thread sleeps while the plugin is off.
const LONGEST_SLEEP: Duration = Duration::from_secs(3600);
/// `ssgw.sh` waits up to 30 s for its lock.
const TOOL_WITHIN: Duration = Duration::from_secs(40);
const NOTIFY_WITHIN: Duration = Duration::from_secs(20);
const CRASHED: &str = "TV: Home Assistant gateway crashed";
const STOPPED: &str = "TV: Home Assistant gateway stopped";
const LG_UNTIL: &str = "LG's gateway serves Home Assistant until the TV restarts or Try again in Settings > System > Home Assistant \
                        gateway.";

/// Where the plugin finds and puts things: the TV's ([`Files::tv`]) or a test's.
struct Files {
    /// `ssgw` and `ssgw-accept`, staged by `lumatv`.
    bin: PathBuf,
    /// The gateway's own: its client store.
    state: PathBuf,
    /// Its sockets, on a tmpfs: `ssgw-accept`, started by xinetd with nothing in its environment, looks for them here.
    run: PathBuf,
    /// The crash records, with `latest`, `pending` and `gave-up`.
    crash: PathBuf,
    /// The app's `ssgw.sh`, `sysbus.sh` and `declutter.sh`.
    tools: PathBuf,
    luna_send: PathBuf,
    boot_id: PathBuf,
    /// Whom the gateway runs as (user, group): its own when the helper is root.
    user: Option<(u32, u32)>,
    /// The sockets the helper connects for a gateway running as its own user (`SSGW_BROKER_FD`); with none, it connects them itself.
    netinput: Vec<PathBuf>,
}

impl Files {
    fn tv(ctx: &Ctx) -> Self {
        // SAFETY: `geteuid` has no preconditions.
        let root = unsafe { libc::geteuid() } == 0;
        let app = ctx.config.brand_app_id.as_deref().unwrap_or("org.nphil.lumatv");
        let dir = ctx.paths.data_dir.join("ssgw");
        Self {
            bin: dir.join("bin"),
            state: dir.join("state"),
            run: PathBuf::from("/tmp/lumatv-ssgw"),
            crash: dir.join("crash"),
            tools: PathBuf::from(format!("/media/developer/apps/usr/palm/applications/{app}/tools")),
            luna_send: PathBuf::from(&ctx.config.luna.send),
            boot_id: PathBuf::from("/proc/sys/kernel/random/boot_id"),
            user: root.then_some((GATEWAY_ID, GATEWAY_ID)),
            netinput: NETINPUT.iter().map(PathBuf::from).collect(),
        }
    }
}

/// Tells the owner's phone `(service, title, message)`: the TV's sends through Home Assistant on a thread of its own and reports with
/// [`Event::Told`]; a test's records.
type Notify = Arc<dyn Fn(&str, &str, &str) + Send + Sync>;

fn text_of(values: &Values, key: &str) -> String {
    match values.get(key) {
        Some(Setting::Text(t)) => t.trim().to_owned(),
        _ => String::new(),
    }
}

/// The page, as the schema format writes it: the notice of a crash and its Mark as read only while one waits, Try again only while
/// the plugin has stopped trying.
fn schema_json() -> Json {
    let fact = |key: &str, label: &str| {
        json!({"key": key, "label": label, "kind": {"type": "read-only", "value": {"capability": format!("plugin.{ID}.{key}")}},
               "apply": "on-commit"})
    };
    let action = |key: &str, label: &str, verb: &str, help: &str| {
        json!({"key": key, "label": label, "help": help, "kind": {"type": "action", "action": key, "confirmation": "none", "verb": verb},
               "apply": "on-commit"})
    };
    let with = |mut field: Json, fact: &str| {
        field["visible_when"] = json!({"type": "capability-present", "capability": format!("plugin.{ID}.{fact}")});
        field
    };
    json!({"format": 1, "owner": ID, "revision": 1, "title": OPT_IN.spec.name, "sections": [
        {"id": "gateway", "title": "Gateway", "fields": [
            fact("status", "Status"),
            with(fact("crash", "Last crash"), "crash"),
            with(fact("crash-log", "Its last error"), "crash"),
            with(fact("crash-where", "Its record"), "crash"),
            with(action(ACK, "Mark the crash as read", "Mark as read", "The record stays on the TV."), "crash"),
            with(action(RETRY, "Try again", "Try again", "Starts LumaTV's gateway again now. LG's serves until it answers."), "gaveup"),
            with(fact("notified", "Phone"), "notified"),
            {"key": NOTIFY, "label": "Tell crashes to",
             "help": "The Home Assistant notify service told of each crash, such as notify.mobile_app_phone. Empty: nobody is told.",
             "kind": {"type": "text", "max_chars": 100, "secret": false},
             "default": {"type": "text", "value": NOTIFY_DEFAULT}, "apply": "on-commit"},
            action(FORGET, "Forget clients", "Forget",
                   "LumaTV's gateway forgets the keys it has served. Each is checked with LG's gateway again on its next connection."),
            fact("sysbus", "LG service host repair"),
            fact("lgextras", "LG extras")]}]})
}

fn register() -> Json {
    json!({"title": OPT_IN.spec.name, "settings": schema_json()})
}

/// What the plugin's thread is told.
enum Event {
    /// The owner's switch may have changed: [`Shared::switched_on`] says how it stands.
    Switch,
    Settings(Values),
    Action(Action),
    /// The gateway's log ended: it has exited.
    Ended,
    /// How the last notification went.
    Told(Result<(), String>),
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    Ack,
    Retry,
    Forget,
}

/// The gateway's answer on its control socket.
#[derive(Clone, Debug, Default, PartialEq)]
struct Pong {
    version: String,
    uptime_s: u64,
    sessions: u64,
    /// The last URIs it served, newest last; no payloads.
    recent: Vec<String>,
}

/// Asks the gateway on its control socket whether it answers: one line each way, within a second.
fn ping(control: &Path) -> Option<Pong> {
    let mut stream = UnixStream::connect(control).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(1))).ok()?;
    stream.write_all(b"ping\n").ok()?;
    let mut line = String::new();
    BufReader::new(stream.take(64 * 1024)).read_line(&mut line).ok()?;
    let v: Json = serde_json::from_str(&line).ok()?;
    if v["ok"] != json!(true) {
        return None;
    }
    let plain = |s: &str, max: usize| clip(&s.replace(char::is_control, " "), max);
    Some(Pong {
        version: plain(v["version"].as_str().unwrap_or(""), 40),
        uptime_s: v["uptimeS"].as_u64().unwrap_or(0),
        sessions: v["sessions"].as_u64().unwrap_or(0),
        recent: v["recent"]
            .as_array()
            .map(|a| a.iter().filter_map(Json::as_str).take(20).map(|u| plain(u, 200)).collect())
            .unwrap_or_default(),
    })
}

/// How a run of the gateway ended without being asked to.
enum Ending {
    Exited(ExitStatus),
    Hung,
    Unstarted(String),
}

impl Ending {
    fn code(&self) -> Option<i32> {
        match self {
            Self::Exited(status) => status.code(),
            _ => None,
        }
    }

    fn words(&self) -> String {
        match self {
            Self::Exited(status) => match (status.code(), status.signal()) {
                (Some(0), _) => "exited on its own".into(),
                (Some(101), _) => "panicked".into(),
                (Some(code), _) => format!("exited with status {code}"),
                (None, Some(signal)) => format!("was killed by signal {signal}{}", signal_name(signal)),
                (None, None) => "ended".into(),
            },
            Self::Hung => format!("stopped answering its health checks for {} s", (TICK * MISSES).as_secs()),
            Self::Unstarted(why) => format!("could not be started ({why})"),
        }
    }
}

fn signal_name(signal: i32) -> &'static str {
    match signal {
        libc::SIGSEGV => " (SIGSEGV)",
        libc::SIGABRT => " (SIGABRT)",
        libc::SIGBUS => " (SIGBUS)",
        libc::SIGILL => " (SIGILL)",
        libc::SIGFPE => " (SIGFPE)",
        libc::SIGKILL => " (SIGKILL)",
        libc::SIGTERM => " (SIGTERM)",
        _ => "",
    }
}

/// A duration as the page and the phone say it: `45 s`, `12 min`, `3 h`, `2 d`.
fn span(d: Duration) -> String {
    match d.as_secs() {
        s @ 0..90 => format!("{s} s"),
        s @ 90..5_400 => format!("{} min", s / 60),
        s @ 5_400..172_800 => format!("{} h", s / 3_600),
        s => format!("{} d", s / 86_400),
    }
}

/// The gateway's last error in its log, or else its last line.
fn last_error(lines: &VecDeque<String>) -> String {
    lines
        .iter()
        .rev()
        .find_map(|l| l.strip_prefix("ERROR "))
        .or_else(|| lines.back().map(String::as_str))
        .unwrap_or("it logged nothing")
        .to_owned()
}

/// A connected pair of sockets that keep each datagram whole, both close-on-exec: the broker's end and the gateway's.
fn seqpacket_pair() -> std::io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: `fds` has room for the two descriptors the call writes.
    if unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0, fds.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: both were just made and nothing else holds them.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

/// Connects LG's input sockets for a gateway running as its own user, a request at a time, until the gateway's end closes. A request
/// is a datagram naming one of `allowed`; the answer is `K` with the connection beside it, or `E` and the errno (13 for anything else).
fn broker(end: &OwnedFd, allowed: &[PathBuf]) {
    // longer than any socket's path, so a longer request is cut short and matches none
    let mut request = [0u8; 128];
    loop {
        // SAFETY: `request` outlives the call and its length is the one given.
        let n = unsafe { libc::recv(end.as_raw_fd(), request.as_mut_ptr().cast(), request.len(), 0) };
        if n < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        let Ok(n) = usize::try_from(n) else { return };
        if n == 0 {
            return;
        }
        let asked = &request[..n];
        let answer = match allowed.iter().find(|path| path.as_os_str().as_bytes() == asked) {
            Some(path) => connect_unix(path),
            None => Err(libc::EACCES),
        };
        let sent = match answer {
            Ok(connection) => send(end, b"K", Some(&connection)),
            Err(errno) => send(end, format!("E{errno}").as_bytes(), None),
        };
        if sent.is_err() {
            return;
        }
    }
}

/// A connection to the stream socket at `path`, or the errno. It is made without waiting on a listener whose queue is full, and handed
/// over blocking, as a socket the gateway made itself would be.
fn connect_unix(path: &Path) -> Result<OwnedFd, i32> {
    let errno = || std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO);
    let name = path.as_os_str().as_bytes();
    // SAFETY: zero is a valid sockaddr_un.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if name.len() >= address.sun_path.len() {
        return Err(libc::ENAMETOOLONG);
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (to, from) in address.sun_path.iter_mut().zip(name) {
        *to = libc::c_char::from_ne_bytes([*from]);
    }
    // SAFETY: no memory is passed.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0) };
    if fd < 0 {
        return Err(errno());
    }
    // SAFETY: `fd` was just made and nothing else holds it.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let length = std::mem::offset_of!(libc::sockaddr_un, sun_path) + name.len() + 1;
    // SAFETY: `address` is a sockaddr_un and `length` lies within it.
    if unsafe { libc::connect(fd.as_raw_fd(), (&raw const address).cast(), length as libc::socklen_t) } != 0 {
        return Err(errno());
    }
    // SAFETY: no memory is passed.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    // SAFETY: no memory is passed.
    if flags < 0 || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK) } < 0 {
        return Err(errno());
    }
    Ok(fd)
}

/// Sends `bytes` as one datagram, with `fd` beside it (SCM_RIGHTS) when there is one.
fn send(end: &OwnedFd, bytes: &[u8], fd: Option<&OwnedFd>) -> std::io::Result<()> {
    let mut iov = libc::iovec { iov_base: bytes.as_ptr().cast_mut().cast(), iov_len: bytes.len() };
    // aligned for a cmsghdr and longer than CMSG_SPACE of one descriptor
    let mut control = [0u64; 4];
    // SAFETY: zero is a valid msghdr; `iov`, `bytes` and `control` outlive the call, and the one header written fits in `control`.
    let sent = unsafe {
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        if let Some(fd) = fd {
            msg.msg_control = control.as_mut_ptr().cast();
            msg.msg_controllen = libc::CMSG_SPACE(size_of::<libc::c_int>() as u32) as _;
            let header = libc::CMSG_FIRSTHDR(&msg);
            (*header).cmsg_level = libc::SOL_SOCKET;
            (*header).cmsg_type = libc::SCM_RIGHTS;
            (*header).cmsg_len = libc::CMSG_LEN(size_of::<libc::c_int>() as u32) as _;
            libc::CMSG_DATA(header).cast::<libc::c_int>().write_unaligned(fd.as_raw_fd());
        }
        libc::sendmsg(end.as_raw_fd(), &msg, libc::MSG_NOSIGNAL)
    };
    if sent < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
}

fn inode(path: &Path) -> Option<u64> {
    std::fs::metadata(path).ok().map(|m| m.ino())
}

/// Runs `command` to its end within `within`: its output (at most 64 KiB), or why there is none.
fn run(mut command: Command, within: Duration) -> Result<String, String> {
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let Some(out) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("no output".into());
    };
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::Builder::new()
        .name("ssgw-tool".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            let mut text = String::new();
            let _ = out.take(64 * 1024).read_to_string(&mut text);
            let _ = tx.send(text);
        });
    if let Err(e) = reader {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("its output: {e}"));
    }
    let text = rx.recv_timeout(within).map_err(|_| {
        let _ = child.kill();
        "it did not finish in time".to_owned()
    });
    let _ = child.wait();
    text
}

/// The gateway's run directory as `ssgw-accept` trusts it: a directory, not a link, owned by the gateway's user, mode 0700. Whatever
/// else is in its place goes first.
fn run_dir(path: &Path, ids: Option<(u32, u32)>) -> std::io::Result<()> {
    // SAFETY: neither call has preconditions.
    let (uid, gid) = ids.unwrap_or_else(|| unsafe { (libc::geteuid(), libc::getegid()) });
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() && m.uid() == uid && m.mode() & 0o7777 == 0o700 => return Ok(()),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path)?,
        Ok(_) => std::fs::remove_file(path)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    std::fs::create_dir(path)?;
    std::os::unix::fs::chown(path, Some(uid), Some(gid))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

/// The gateway's state directory (mode 0700) and what is in it, its user's: a run as root may have left files of root's.
fn state_dir(path: &Path, ids: Option<(u32, u32)>) -> std::io::Result<()> {
    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    if let Some((uid, gid)) = ids {
        std::os::unix::fs::lchown(path, Some(uid), Some(gid))?;
        for entry in std::fs::read_dir(path)? {
            std::os::unix::fs::lchown(entry?.path(), Some(uid), Some(gid))?;
        }
    }
    Ok(())
}

/// Reads the gateway's log into `ring` (its last [`LOG_LINES`]) and the helper's log at the level it gives (a line said again in a
/// row once), then waits for the gateway to exit and says so. `done` goes with the thread.
fn log_reader(
    log: Option<ChildStderr>,
    pid: u32,
    ring: Arc<Mutex<VecDeque<String>>>,
    tx: Sender<Event>,
    done: Sender<()>,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("ssgw-log".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            let _done = done;
            if let Some(log) = log {
                let mut log = BufReader::new(log);
                let (mut bytes, mut last) = (Vec::new(), String::new());
                loop {
                    bytes.clear();
                    match (&mut log).take(LOG_LINE_BYTES).read_until(b'\n', &mut bytes) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                    let line = String::from_utf8_lossy(&bytes).replace(char::is_control, " ").trim_end().to_owned();
                    if line != last {
                        match line.split_once(' ') {
                            Some(("ERROR", said)) => crate::log_error!("ssgw: {said}"),
                            Some(("WARN", said)) => crate::log_warn!("ssgw: {said}"),
                            Some(("DEBUG", said)) => crate::log_debug!("ssgw: {said}"),
                            Some(("INFO", said)) => crate::log_info!("ssgw: {said}"),
                            _ => crate::log_info!("ssgw: {line}"),
                        }
                    }
                    let mut ring = ring.lock();
                    if ring.len() == LOG_LINES {
                        ring.pop_front();
                    }
                    ring.push_back(line.clone());
                    drop(ring);
                    last = line;
                }
            }
            // The log closes as the gateway exits, a moment before the exit can be collected: waiting for that (without collecting
            // it) lets the thread see the end now rather than at its next health check.
            // SAFETY: `info` is written by the call; WNOWAIT leaves the child to `Child::try_wait`.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            unsafe { libc::waitid(libc::P_PID, pid, &mut info, libc::WEXITED | libc::WNOWAIT) };
            let _ = tx.send(Event::Ended);
        })
}

/// A run of the gateway.
struct Running {
    child: Child,
    /// Closed when its log has been read to the end.
    log_done: Receiver<()>,
    started: Instant,
    /// The program's inode when it was started: another staged in its place restarts the gateway.
    inode: u64,
    next_ping: Instant,
    misses: u32,
    pong: Option<Pong>,
    as_root: bool,
}

/// A crash as the page shows it until it is marked read.
#[derive(Clone, Debug, PartialEq)]
struct Notice {
    line: String,
    error: String,
    place: String,
}

/// The crash whose notice waits to be read (`pending` names its record).
fn pending(crash: &Path) -> Option<Notice> {
    let name = std::fs::read_to_string(crash.join("pending")).ok()?.trim().to_owned();
    if name.is_empty() || name.contains('/') || name.contains("..") {
        return None;
    }
    let dir = crash.join(&name);
    let reason = std::fs::read_to_string(dir.join("reason.txt")).ok()?;
    let field = |key: &str| reason.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(": ")).unwrap_or("").to_owned();
    Some(Notice {
        line: field("summary"),
        error: format!("Last error: {}", field("last error")),
        place: format!("Kept on the TV in {}", dir.display()),
    })
}

/// Removes all but the newest `keep` records; their names sort by time.
fn prune(crash: &Path, keep: usize) {
    let Ok(dir) = std::fs::read_dir(crash) else { return };
    let mut records: Vec<PathBuf> = dir.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    records.sort();
    for old in records.iter().rev().skip(keep) {
        let _ = std::fs::remove_dir_all(old);
    }
}

/// The gateway's supervisor. Time is given to it, so a test runs it on a clock of its own.
struct Gateway {
    files: Files,
    notify: Notify,
    /// Where the log reader says the gateway ended.
    tx: Sender<Event>,
    log: Arc<Mutex<VecDeque<String>>>,
    running: Option<Running>,
    /// When the gateway is started (again), while it is not running.
    due: Instant,
    /// Crashes in a row, for the pause.
    failures: u32,
    /// When the recent crashes were, for giving up.
    crashes: Vec<Instant>,
    /// Why the plugin stopped trying, until the next boot or Try again.
    gave_up: Option<String>,
    /// Whether the gateway runs as root.
    as_root: bool,
    /// Why port 3001 is not handed to the gateway, when it is not.
    unbound: Option<String>,
    notice: Option<Notice>,
    /// The notify service, from the page.
    service: String,
    /// Why the last notification did not go.
    untold: Option<String>,
    /// An earlier gateway still answered when this one was due.
    waiting: bool,
    sysbus: String,
    lgextras: String,
    next_side: Instant,
}

impl Gateway {
    fn new(files: Files, notify: Notify, tx: Sender<Event>, now: Instant) -> Self {
        Self {
            notice: pending(&files.crash),
            files,
            notify,
            tx,
            log: Arc::default(),
            running: None,
            due: now,
            failures: 0,
            crashes: Vec::new(),
            gave_up: None,
            as_root: false,
            unbound: None,
            service: String::new(),
            untold: None,
            waiting: false,
            sysbus: String::new(),
            lgextras: String::new(),
            next_side: now,
        }
    }

    fn installed(&self) -> bool {
        self.files.bin.join("ssgw").is_file() && self.files.bin.join("ssgw-accept").is_file()
    }

    fn boot_id(&self) -> String {
        std::fs::read_to_string(&self.files.boot_id).map(|b| b.trim().to_owned()).unwrap_or_default()
    }

    /// One of the app's tools, as the JSON line it ends with; a refusal (`"ok": false`) as its words.
    fn tool(&self, name: &str, args: &[&str]) -> Result<Json, String> {
        let mut command = Command::new("/bin/sh");
        command.arg(self.files.tools.join(name)).args(args);
        let out = run(command, TOOL_WITHIN).map_err(|e| format!("{name}: {e}"))?;
        let line = out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
        let v: Json = serde_json::from_str(line).map_err(|_| format!("{name} said nothing readable"))?;
        if v["ok"] == json!(true) {
            Ok(v)
        } else {
            Err(v["error"].as_str().unwrap_or("it refused").to_owned())
        }
    }

    /// Switched on: port 3001 handed over, the directories laid out, whom the gateway runs as decided, and a stop of this boot read
    /// back.
    fn on(&mut self, now: Instant) {
        if let Err(e) = self.prepare() {
            crate::log_warn!("ssgw: {}: {e}", self.files.crash.display());
        }
        self.bind();
        self.as_root = self.files.user.is_some() && !self.luna_works();
        if self.as_root {
            crate::log_warn!("ssgw: luna-send does not answer the gateway's own user; the gateway runs as root");
        }
        self.gave_up = self.gave_up_this_boot();
        self.crashes.clear();
        self.failures = 0;
        self.due = now;
        self.next_side = now;
    }

    /// The data directory, passed through to the gateway's state but not read by it (0711), and the crash records'.
    fn prepare(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.files.crash)?;
        if let Some(dir) = self.files.state.parent() {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o711))?;
        }
        Ok(())
    }

    /// Has `ssgw.sh on` hand port 3001's connections to `ssgw-accept`.
    fn bind(&mut self) {
        self.unbound = match self.tool("ssgw.sh", &["on"]) {
            Ok(said) => {
                if said["ssgw"] == "bound" {
                    crate::log_info!("ssgw: port 3001's connections go to LumaTV's gateway first");
                }
                None
            }
            Err(e) => {
                crate::log_warn!("ssgw: port 3001 was not handed over: {e}");
                Some(e)
            }
        };
    }

    /// Whether `luna-send` answers when run as the gateway's user.
    fn luna_works(&self) -> bool {
        let Some(ids) = self.files.user else { return true };
        let mut command = Command::new(&self.files.luna_send);
        command.args(["-n", "1", "luna://com.webos.service.tvpower/power/getPowerState", "{}"]);
        confine(&mut command, Some(ids), None);
        run(command, Duration::from_secs(10))
            .ok()
            .and_then(|out| serde_json::from_str::<Json>(out.trim()).ok())
            .is_some_and(|v| v["returnValue"] == json!(true))
    }

    fn gave_up_this_boot(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.files.crash.join("gave-up")).ok()?;
        let (boot, why) = text.split_once('\n')?;
        (boot.trim() == self.boot_id()).then(|| why.trim().to_owned())
    }

    /// Switched off: the gateway stopped and port 3001 given back to LG's entry.
    fn off(&mut self) {
        self.halt();
        match self.tool("ssgw.sh", &["off"]) {
            Ok(_) => crate::log_info!("ssgw: port 3001 is LG's gateway's again"),
            Err(e) => crate::log_warn!("ssgw: port 3001 was not given back to LG's gateway: {e}"),
        }
        self.unbound = None;
        self.waiting = false;
    }

    /// Stops the gateway as asked: no crash.
    fn halt(&mut self) {
        if let Some(running) = self.running.take() {
            end(running.child, GRACE);
        }
    }

    /// When the thread has something to do next.
    fn wake_at(&self) -> Instant {
        match &self.running {
            Some(r) => self.next_side.min(r.next_ping),
            None if self.gave_up.is_none() && self.installed() => self.next_side.min(self.due),
            None => self.next_side,
        }
    }

    fn step(&mut self, now: Instant) {
        self.watch(now);
        if self.running.is_none() && self.gave_up.is_none() && now >= self.due {
            self.start(now);
        }
        if now >= self.next_side {
            self.side();
            self.next_side = now + SIDE_EVERY;
        }
    }

    /// Notices the gateway's exit, a new program staged in its place, and whether it still answers.
    fn watch(&mut self, now: Instant) {
        let Some(r) = self.running.as_mut() else { return };
        if let Ok(Some(status)) = r.child.try_wait() {
            if let Some(r) = self.running.take() {
                self.ended(r, Ending::Exited(status), now);
            }
            return;
        }
        let (started_inode, ping_due) = (r.inode, now >= r.next_ping);
        if inode(&self.files.bin.join("ssgw")).is_some_and(|i| i != started_inode) {
            crate::log_info!("ssgw: a new gateway was staged; starting it");
            self.halt();
            self.due = now;
            return;
        }
        if !ping_due {
            return;
        }
        let pong = ping(&self.files.run.join("control.sock"));
        let Some(r) = self.running.as_mut() else { return };
        r.next_ping = now + TICK;
        match pong {
            Some(pong) => {
                if r.pong.is_none() {
                    crate::log_info!("ssgw: the gateway answers (version {})", pong.version);
                }
                r.misses = 0;
                r.pong = Some(pong);
            }
            None => {
                r.misses += 1;
                if r.misses >= MISSES
                    && let Some(mut r) = self.running.take()
                {
                    sweep(Some(r.child.id()));
                    let _ = r.child.kill();
                    let _ = r.child.wait();
                    self.ended(r, Ending::Hung, now);
                }
            }
        }
    }

    fn ended(&mut self, mut r: Running, how: Ending, now: Instant) {
        // what it started, and the rest of its log
        sweep(Some(r.child.id()));
        let _ = r.log_done.recv_timeout(Duration::from_secs(1));
        let ran = now.saturating_duration_since(r.started);
        self.crashed(how, ran, r.pong.take(), r.as_root, now);
    }

    fn start(&mut self, now: Instant) {
        if !self.installed() {
            return;
        }
        let control = self.files.run.join("control.sock");
        // A gateway this helper did not start (a helper before this one, stopping) would make this one refuse to start: no crash.
        if ping(&control).is_some() {
            if !self.waiting {
                crate::log_warn!("ssgw: an earlier gateway still answers on {}; waiting for it", control.display());
            }
            self.waiting = true;
            self.due = now + FIRST_PAUSE;
            return;
        }
        self.waiting = false;
        match self.spawn(now) {
            Ok(running) => self.running = Some(running),
            Err(why) => self.crashed(Ending::Unstarted(why), Duration::ZERO, None, self.as_root, now),
        }
    }

    fn spawn(&mut self, now: Instant) -> Result<Running, String> {
        let ids = if self.as_root { None } else { self.files.user };
        run_dir(&self.files.run, ids).map_err(|e| format!("{}: {e}", self.files.run.display()))?;
        state_dir(&self.files.state, ids).map_err(|e| format!("{}: {e}", self.files.state.display()))?;
        let program = self.files.bin.join("ssgw");
        let inode = inode(&program).ok_or("it is not installed")?;
        let mut command = Command::new(&program);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("SSGW_STATE_DIR", &self.files.state)
            .env("SSGW_RUN_DIR", &self.files.run)
            .env("SSGW_LOG", "info")
            .current_dir(&self.files.state)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        confine_files(&mut command, ids, None, FILES);
        let broker_ends = if self.as_root || self.files.netinput.is_empty() {
            None
        } else {
            Some(seqpacket_pair().map_err(|e| format!("its broker: {e}"))?)
        };
        if let Some((_, theirs)) = &broker_ends {
            let fd = theirs.as_raw_fd();
            command.env("SSGW_BROKER_FD", fd.to_string());
            // SAFETY: `fcntl` is async-signal-safe and the closure touches nothing else.
            unsafe {
                command.pre_exec(move || {
                    if libc::fcntl(fd, libc::F_SETFD, 0) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        if let Some((ours, theirs)) = broker_ends {
            // the gateway's end is left to the gateway alone, so the broker sees its requests end when the gateway exits
            drop(theirs);
            let allowed = self.files.netinput.clone();
            let started = std::thread::Builder::new()
                .name("ssgw-broker".into())
                .stack_size(64 * 1024)
                .spawn(move || broker(&ours, &allowed));
            if let Err(e) = started {
                end(child, Duration::ZERO);
                return Err(format!("its broker: {e}"));
            }
        }
        self.log.lock().clear();
        let (done, log_done) = mpsc::channel();
        if let Err(e) = log_reader(child.stderr.take(), child.id(), self.log.clone(), self.tx.clone(), done) {
            end(child, Duration::ZERO);
            return Err(format!("its log: {e}"));
        }
        Ok(Running {
            child,
            log_done,
            started: now,
            inode,
            next_ping: now + TICK,
            misses: 0,
            pong: None,
            as_root: self.as_root,
        })
    }

    /// The gateway ended without being asked, or could not be started: kept, shown and told once, and started again after a pause, or
    /// not until the next boot or Try again.
    fn crashed(&mut self, how: Ending, ran: Duration, pong: Option<Pong>, as_root: bool, now: Instant) {
        if how.code() == Some(UNUSABLE) {
            let detail = last_error(&self.log.lock());
            if !as_root && self.files.user.is_some() {
                crate::log_warn!("ssgw: the gateway cannot work as its own user ({detail}); it runs as root");
                self.as_root = true;
                self.due = now;
                return;
            }
            let why = format!("it cannot serve this TV ({detail})");
            crate::log_warn!("ssgw: {why}");
            self.keep("unusable", &why, ran, pong.as_ref(), as_root);
            self.give_up("it cannot serve this TV".into());
            self.tell(STOPPED, &format!("LumaTV's gateway on the TV says {why}. {LG_UNTIL}"));
            return;
        }
        let words = how.words();
        crate::log_warn!("ssgw: the gateway {words} after {}", span(ran));
        if ran >= STEADY {
            self.failures = 0;
        }
        self.crashes.retain(|t| now.saturating_duration_since(*t) < WINDOW);
        self.crashes.push(now);
        if self.crashes.len() >= CRASHES {
            let within = format!("{CRASHES} crashes within {} min", WINDOW.as_secs() / 60);
            self.keep("gave-up", &words, ran, pong.as_ref(), as_root);
            self.give_up(within.clone());
            self.tell(STOPPED, &format!("LumaTV's gateway on the TV {words} after {}: {within}. {LG_UNTIL}", span(ran)));
        } else {
            let pause = self.pause();
            self.due = now + pause;
            self.keep("crash", &words, ran, pong.as_ref(), as_root);
            self.tell(
                CRASHED,
                &format!(
                    "LumaTV's gateway on the TV {words} after {}. LG's gateway is serving Home Assistant; LumaTV's starts again in {}.",
                    span(ran),
                    span(pause)
                ),
            );
        }
    }

    /// The pause before the next start, longer each time in a row.
    fn pause(&mut self) -> Duration {
        let pause = FIRST_PAUSE.saturating_mul(1 << self.failures.min(5)).min(LAST_PAUSE);
        self.failures += 1;
        pause
    }

    /// Keeps a crash under `crash/<time>-<n>/` (`reason.txt`, the log's last lines, the last URIs served), the newest [`RECORDS`] of
    /// them, and shows it until it is marked read (`latest` and `pending`, as `crash.rs` has the compositor's).
    fn keep(&mut self, kind: &str, how: &str, ran: Duration, pong: Option<&Pong>, as_root: bool) {
        let stamp = utc_stamp(now_ms());
        let digits: String = stamp.chars().filter(char::is_ascii_digit).collect();
        let base = format!("{}-{}", &digits[..8], &digits[8..14]);
        let Some((name, dir)) = (1..1000)
            .map(|n| format!("{base}-{n:03}"))
            .map(|name| {
                let dir = self.files.crash.join(&name);
                (name, dir)
            })
            .find(|(_, dir)| !dir.exists())
        else {
            return;
        };
        let log: VecDeque<String> = self.log.lock().clone();
        let error = last_error(&log);
        let summary = format!("{} UTC: {how} after {}", stamp[..16].replace('T', " "), span(ran));
        let reason = format!(
            "reason: {how}\nkind: {kind}\ntime: {stamp}\ngateway uptime: {} s\nboot: {}\nfailures: {}\nsessions: {}\nversion: {}\n\
             user: {}\nsummary: {summary}\nlast error: {error}\n",
            ran.as_secs(),
            self.boot_id(),
            self.crashes.len(),
            pong.map_or(0, |p| p.sessions),
            pong.map_or("", |p| p.version.as_str()),
            if as_root { "root" } else { "gateway" },
        );
        let written = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&dir)?;
            std::fs::write(dir.join("reason.txt"), reason)?;
            std::fs::write(dir.join("gateway.log.tail"), log.iter().map(|l| format!("{l}\n")).collect::<String>())?;
            let recent = pong.map(|p| p.recent.iter().map(|u| format!("{u}\n")).collect::<String>()).unwrap_or_default();
            std::fs::write(dir.join("recent.txt"), recent)?;
            std::fs::write(self.files.crash.join("latest"), &name)?;
            std::fs::write(self.files.crash.join("pending"), &name)
        })();
        if let Err(e) = written {
            crate::log_warn!("ssgw: the crash was not kept in {}: {e}", dir.display());
        }
        prune(&self.files.crash, RECORDS);
        self.notice = Some(Notice {
            line: summary,
            error: format!("Last error: {error}"),
            place: format!("Kept on the TV in {}", dir.display()),
        });
    }

    /// Stops trying until the next boot or Try again; kept beside the records, so a helper started again in this boot knows.
    fn give_up(&mut self, why: String) {
        let kept = std::fs::create_dir_all(&self.files.crash)
            .and_then(|()| std::fs::write(self.files.crash.join("gave-up"), format!("{}\n{why}\n", self.boot_id())));
        if let Err(e) = kept {
            crate::log_warn!("ssgw: the stop was not kept: {e}");
        }
        crate::log_warn!("ssgw: stopped trying ({why}); LG's gateway serves until the next boot or Try again");
        self.gave_up = Some(why);
    }

    fn tell(&self, title: &str, message: &str) {
        if !self.service.is_empty() {
            (self.notify)(&self.service, title, message);
        }
    }

    fn act(&mut self, action: Action, now: Instant) {
        match action {
            Action::Ack => {
                let _ = std::fs::remove_file(self.files.crash.join("pending"));
                self.notice = None;
            }
            Action::Retry => {
                let _ = std::fs::remove_file(self.files.crash.join("gave-up"));
                if self.gave_up.take().is_some() {
                    crate::log_info!("ssgw: trying the gateway again");
                }
                self.crashes.clear();
                self.failures = 0;
                self.due = now;
                self.next_side = now;
            }
            Action::Forget => {
                self.halt();
                match std::fs::remove_file(self.files.state.join("clients.json")) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                        crate::log_warn!("ssgw: the gateway's clients were not forgotten: {e}");
                    }
                    _ => crate::log_info!("ssgw: the gateway's clients were forgotten; LG's gateway checks each again"),
                }
                self.due = now;
            }
        }
    }

    /// The page's other lines: whether port 3001 is still handed over (it is handed over again when it is not), LG's service host's
    /// repair, which decides what the fallback costs, and LG's extras.
    fn side(&mut self) {
        match self.tool("ssgw.sh", &["status"]) {
            Ok(said) if said["entry"] != "ours" => self.bind(),
            Ok(_) => self.unbound = None,
            Err(e) => self.unbound = Some(e),
        }
        self.sysbus = match self.tool("sysbus.sh", &["status"]) {
            Ok(said) if said["launcher"] == "ours" => "On: LG's gateway as the fallback is light".into(),
            Ok(_) => "Off: LG's gateway as the fallback costs about 22 % of a core".into(),
            Err(e) => format!("Unknown: {e}"),
        };
        self.lgextras = match self.tool("declutter.sh", &["status", "--group", "lgextras"]) {
            Ok(said) => match said.pointer("/groups/lgextras/on") {
                Some(Json::Bool(true)) => "Off: LumaTV keeps them from loading".into(),
                Some(Json::Bool(false)) => "On: they keep LG's service host running".into(),
                _ => "Unknown".into(),
            },
            Err(e) => format!("Unknown: {e}"),
        };
    }

    fn status(&self) -> String {
        if !self.installed() {
            return "Not installed: this package has no gateway, and LG's serves Home Assistant".into();
        }
        if let Some(why) = &self.gave_up {
            return format!("Stopped after {why}; LG's gateway is serving");
        }
        if let Some(why) = &self.unbound {
            return format!("Not serving: port 3001 is LG's ({why})");
        }
        match &self.running {
            Some(r) if r.misses > 0 => {
                format!("Not answering ({} of {MISSES} checks missed); LG's gateway takes new connections", r.misses)
            }
            Some(Running { pong: Some(p), .. }) => format!(
                "Serving Home Assistant: {} session{}, up {}",
                p.sessions,
                if p.sessions == 1 { "" } else { "s" },
                span(Duration::from_secs(p.uptime_s))
            ),
            Some(_) => "Starting".into(),
            None if self.waiting => "Waiting for an earlier gateway to stop".into(),
            None => "Restarting after a crash; LG's gateway is serving meanwhile".into(),
        }
    }

    fn facts(&self) -> Map<String, Json> {
        let mut facts = Map::new();
        let mut put = |key: &str, line: &str| {
            if !line.is_empty() {
                facts.insert(key.into(), json!(clip(&line.replace(char::is_control, " "), MAX_FACT_LINE)));
            }
        };
        put("status", &self.status());
        if let Some(n) = &self.notice {
            put("crash", &n.line);
            put("crash-log", &n.error);
            put("crash-where", &n.place);
        }
        if let Some(why) = &self.gave_up {
            put("gaveup", why);
        }
        if let Some(why) = &self.untold {
            put("notified", &format!("Not told of the last crash: {why}"));
        }
        put("sysbus", &self.sysbus);
        put("lgextras", &self.lgextras);
        facts
    }
}

/// One notification through Home Assistant's `notify` service `service` (`notify.<name>`), over the TV's saved link.
fn notify(links: &[PathBuf], config: &Config, policy: AddressPolicy, service: &str, title: &str, message: &str) -> Result<(), String> {
    let name = service
        .strip_prefix("notify.")
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
        .ok_or_else(|| format!("{service} is not a notify service"))?;
    let link = links
        .iter()
        .find_map(|file| ha_store::load(file).ok().flatten())
        .ok_or("the TV has no Home Assistant link")?;
    let deadline = Instant::now() + NOTIFY_WITHIN;
    let tls = crate::voice::tls_for(&config.fetch);
    let mut ha = Ha::connect(&link, policy, tls.as_ref(), deadline).map_err(|e| e.to_string())?;
    let message = json!({"id": 1, "type": "call_service", "domain": "notify", "service": name,
                         "service_data": {"title": title, "message": message}});
    let said = (|| {
        ha.send_json(&message, deadline).map_err(|e| e.to_string())?;
        loop {
            match ha.recv(deadline.saturating_duration_since(Instant::now())).map_err(|e| e.to_string())? {
                Some(m) if m["id"] == message["id"] && m["type"] == "result" => {
                    return if m["success"] == json!(true) {
                        Ok(())
                    } else {
                        Err(format!("Home Assistant: {}", clip(m["error"]["message"].as_str().unwrap_or("it refused"), 120)))
                    };
                }
                Some(_) => {}
                None => return Err("Home Assistant did not answer in time".into()),
            }
        }
    })();
    ha.close();
    said
}

/// The TV's [`Notify`]: through Home Assistant on a thread of its own, so a slow Home Assistant never holds up the supervision.
fn phone(ctx: &Ctx, policy: AddressPolicy, tx: Sender<Event>) -> Notify {
    let links = vec![ctx.paths.ha_link_file(), ctx.paths.data_dir.join("voice").join("ha.json")];
    let config = ctx.config.clone();
    Arc::new(move |service: &str, title: &str, message: &str| {
        let (links, config, tx) = (links.clone(), config.clone(), tx.clone());
        let (service, title, message) = (service.to_owned(), title.to_owned(), message.to_owned());
        let spawned = std::thread::Builder::new()
            .name("ssgw-notify".into())
            .stack_size(512 * 1024)
            .spawn(move || {
                let told = notify(&links, &config, policy, &service, &title, &message);
                if let Err(e) = &told {
                    crate::log_warn!("ssgw: the phone was not told of the crash: {e}");
                }
                let _ = tx.send(Event::Told(told));
            });
        if let Err(e) = spawned {
            crate::log_warn!("ssgw: the phone was not told of the crash: {e}");
        }
    })
}

/// Offers the plugin (off until the owner switches it on) and starts its thread.
pub fn spawn(ctx: &Ctx, shared: Arc<Shared>, policy: AddressPolicy) -> std::io::Result<JoinHandle<()>> {
    let (tx, rx) = mpsc::channel();
    let notify = phone(ctx, policy, tx.clone());
    spawn_with(ctx, shared, Files::tv(ctx), notify, tx, rx)
}

fn spawn_with(
    ctx: &Ctx,
    shared: Arc<Shared>,
    files: Files,
    notify: Notify,
    tx: Sender<Event>,
    rx: Receiver<Event>,
) -> std::io::Result<JoinHandle<()>> {
    {
        let tx = tx.clone();
        ctx.stop.on_stop(move || {
            let _ = tx.send(Event::Stop);
        });
    }
    let mut plugin = Plugin {
        ctx: ctx.clone(),
        shared: shared.clone(),
        gateway: Gateway::new(files, notify, tx.clone(), Instant::now()),
        tx: tx.clone(),
        page: None,
        on: false,
        said: Json::Null,
    };
    shared.offer(
        &OPT_IN,
        Arc::new(move || {
            let _ = tx.send(Event::Switch);
        }),
    );
    crate::plugins::guard::spawn("ssgw", 256 * 1024, shared, &[ID], ctx.stop.clone(), move |again| {
        crate::sys::bulk_thread();
        if again {
            plugin.follow_afresh();
        }
        plugin.run(&rx)
    })
}

struct Plugin {
    ctx: Ctx,
    shared: Arc<Shared>,
    tx: Sender<Event>,
    gateway: Gateway,
    /// Its conversation with the broker's session while it is on: its page and its facts.
    page: Option<Arc<Local>>,
    on: bool,
    /// The facts last put: unchanged ones are not put again.
    said: Json,
}

impl Plugin {
    /// After a panic: follows the switch from scratch, the gateway being as the panic left it.
    fn follow_afresh(&mut self) {
        self.on = !self.shared.switched_on(ID);
        self.switch();
    }

    fn run(&mut self, rx: &Receiver<Event>) {
        loop {
            let wait = if self.on {
                self.gateway.wake_at().saturating_duration_since(Instant::now())
            } else {
                LONGEST_SLEEP
            };
            match rx.recv_timeout(wait) {
                Ok(Event::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Event::Switch) => self.switch(),
                Ok(Event::Settings(values)) => self.gateway.service = text_of(&values, NOTIFY),
                Ok(Event::Action(action)) if self.on => self.gateway.act(action, Instant::now()),
                Ok(Event::Told(told)) => self.gateway.untold = told.err(),
                Ok(Event::Action(_) | Event::Ended) | Err(RecvTimeoutError::Timeout) => {}
            }
            if self.ctx.stop.is_set() {
                break;
            }
            if self.on {
                self.gateway.step(Instant::now());
            }
            self.publish();
        }
        // The entry stays: connections go to LG's gateway until the helper is back.
        self.gateway.halt();
        self.page = None;
    }

    /// Follows the owner's switch: on registers the page, hands port 3001 over and starts the gateway; off stops it and gives the port
    /// back.
    fn switch(&mut self) {
        let on = self.shared.switched_on(ID);
        if on == self.on {
            return;
        }
        self.on = on;
        if on {
            match Local::start(&self.shared, &OPT_IN.spec, register(), handler(self.tx.clone())) {
                Ok(page) => self.page = Some(page),
                Err(e) => crate::log_warn!("ssgw: the plugin's page was not registered: {e}"),
            }
            self.gateway.service = text_of(&self.ctx.settings.effective(ID).unwrap_or_default(), NOTIFY);
            self.gateway.on(Instant::now());
        } else {
            self.gateway.off();
            self.page = None;
            self.said = Json::Null;
        }
    }

    /// Puts the page's facts when they changed.
    fn publish(&mut self) {
        let Some(page) = &self.page else { return };
        let said = json!({"t": "facts.put", "facts": self.gateway.facts()});
        if said != self.said {
            if let Err(e) = page.say(said.clone()) {
                crate::log_warn!("ssgw: the page's facts were not put: {e}");
            }
            self.said = said;
        }
    }
}

/// Takes what the base says to the thread: the page's values and its actions, answered at once.
fn handler(tx: Sender<Event>) -> Handler {
    Arc::new(move |local: &Local, message: ToPlugin| match message {
        ToPlugin::ActionInvoke { op, action, .. } => {
            let action = match action.as_str() {
                ACK => Some(Action::Ack),
                RETRY => Some(Action::Retry),
                FORGET => Some(Action::Forget),
                _ => None,
            };
            let outcome = match action {
                Some(action) => {
                    let _ = tx.send(Event::Action(action));
                    Ok(())
                }
                None => Err("no such action"),
            };
            let _ = local.answer(op, outcome);
        }
        ToPlugin::SettingsApply { txn, values, .. } => {
            let applied = json!({"t": "settings.result", "txn": txn, "outcome": {"type": "applied", "values": values}});
            let _ = tx.send(Event::Settings(values));
            let _ = local.say(applied);
        }
        _ => {}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use crate::control::Stop;
    use crate::hub::Hub;
    use crate::plugins::Plugins;
    use crate::settings::SettingsStore;
    use std::os::unix::net::UnixListener;

    type Told = Arc<Mutex<Vec<(String, String, String)>>>;

    const SLEEPS: &str = "#!/bin/sh\nexec sleep 30\n";

    /// A TV's places under `dir`: the gateway as `script`, and tools that answer as on a TV and note what they were asked in `calls`.
    fn files(dir: &Path, script: &str) -> Files {
        let bin = dir.join("ssgw/bin");
        let tools = dir.join("tools");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&tools).unwrap();
        let calls = dir.join("calls");
        let ssgw_sh = format!(
            "#!/bin/sh\necho \"ssgw.sh $*\" >> '{}'\ncase $1 in status) echo '{{\"ok\":true,\"entry\":\"ours\"}}' ;; \
             *) echo '{{\"ok\":true,\"ssgw\":\"bound\"}}' ;; esac\n",
            calls.display()
        );
        for (path, text) in [
            (bin.join("ssgw"), script),
            (bin.join("ssgw-accept"), "#!/bin/sh\n"),
            (tools.join("ssgw.sh"), &ssgw_sh),
            (tools.join("sysbus.sh"), "#!/bin/sh\necho '{\"ok\":true,\"launcher\":\"ours\",\"loaded\":\"patched\"}'\n"),
            (tools.join("declutter.sh"), "#!/bin/sh\necho '{\"ok\":true,\"groups\":{\"lgextras\":{\"on\":true}}}'\n"),
        ] {
            // a new file: a gateway running the old one keeps it
            let _ = std::fs::remove_file(&path);
            std::fs::write(&path, text).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        if !dir.join("boot_id").exists() {
            std::fs::write(dir.join("boot_id"), "boot-1\n").unwrap();
        }
        Files {
            bin,
            state: dir.join("ssgw/state"),
            run: dir.join("run"),
            crash: dir.join("ssgw/crash"),
            tools,
            luna_send: PathBuf::from("/bin/false"),
            boot_id: dir.join("boot_id"),
            user: None,
            netinput: Vec::new(),
        }
    }

    fn told_to(told: &Told) -> Notify {
        let told = told.clone();
        Arc::new(move |service: &str, title: &str, message: &str| told.lock().push((service.into(), title.into(), message.into())))
    }

    fn gateway(dir: &Path, script: &str) -> (Gateway, Told) {
        let told = Told::default();
        let (tx, _) = mpsc::channel();
        let mut gateway = Gateway::new(files(dir, script), told_to(&told), tx, Instant::now());
        gateway.service = "notify.mobile_app_phone".into();
        (gateway, told)
    }

    fn pid(gateway: &Gateway) -> u32 {
        gateway.running.as_ref().expect("running").child.id()
    }

    /// Until the gateway has exited, leaving the exit to the supervisor to collect.
    fn exited(gateway: &Gateway) {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is written by the call; WNOWAIT leaves the child to `Child::try_wait`.
        unsafe { libc::waitid(libc::P_PID, pid(gateway), &mut info, libc::WEXITED | libc::WNOWAIT) };
    }

    fn gone(pid: u32) -> bool {
        // SAFETY: signal 0 only asks whether the process exists.
        unsafe { libc::kill(pid as libc::pid_t, 0) != 0 }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    fn record(dir: &Path) -> PathBuf {
        let crash = dir.join("ssgw/crash");
        crash.join(read(&crash.join("pending")).trim())
    }

    #[test]
    fn a_crash_is_kept_shown_and_told_once_and_the_gateway_starts_again_after_the_pause() {
        let dir = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\necho 'INFO listening' >&2\necho 'ERROR the bus went away' >&2\nexec sleep 30\n";
        let (mut gw, told) = gateway(dir.path(), script);
        let t0 = Instant::now();
        gw.on(t0);
        gw.step(t0);
        assert_eq!(gw.facts()["status"], "Starting");
        assert_eq!(gw.facts()["sysbus"], "On: LG's gateway as the fallback is light");
        assert_eq!(gw.facts()["lgextras"], "Off: LumaTV keeps them from loading");
        let first = pid(&gw);

        let control = UnixListener::bind(dir.path().join("run/control.sock")).unwrap();
        let answers = std::thread::spawn(move || {
            let (mut stream, _) = control.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap()).read_line(&mut line).unwrap();
            assert_eq!(line, "ping\n");
            stream
                .write_all(b"{\"ok\":true,\"version\":\"0.1.0\",\"uptimeS\":10,\"sessions\":1,\"recent\":[\"ssap://audio/getVolume\"]}\n")
                .unwrap();
        });
        gw.step(t0 + TICK);
        answers.join().unwrap();
        assert_eq!(gw.facts()["status"], "Serving Home Assistant: 1 session, up 10 s");

        // SAFETY: a signal to the test's own child.
        unsafe { libc::kill(first as libc::pid_t, libc::SIGSEGV) };
        exited(&gw);
        let crashed = t0 + TICK + Duration::from_secs(1);
        gw.step(crashed);
        let told = told.lock().clone();
        assert_eq!(told.len(), 1, "{told:?}");
        assert_eq!((told[0].0.as_str(), told[0].1.as_str()), ("notify.mobile_app_phone", CRASHED));
        assert_eq!(
            told[0].2,
            "LumaTV's gateway on the TV was killed by signal 11 (SIGSEGV) after 11 s. LG's gateway is serving Home Assistant; \
             LumaTV's starts again in 2 s."
        );
        let facts = gw.facts();
        assert!(facts["crash"].as_str().unwrap().ends_with("UTC: was killed by signal 11 (SIGSEGV) after 11 s"), "{facts:?}");
        assert_eq!(facts["crash-log"], "Last error: the bus went away");
        assert_eq!(facts["status"], "Restarting after a crash; LG's gateway is serving meanwhile");
        let kept = record(dir.path());
        let reason = read(&kept.join("reason.txt"));
        for want in ["reason: was killed by signal 11 (SIGSEGV)", "kind: crash", "gateway uptime: 11 s", "sessions: 1", "version: 0.1.0"] {
            assert!(reason.lines().any(|l| l == want), "{want}: {reason}");
        }
        assert_eq!(read(&kept.join("gateway.log.tail")), "INFO listening\nERROR the bus went away\n");
        assert_eq!(read(&kept.join("recent.txt")), "ssap://audio/getVolume\n");

        gw.step(crashed + Duration::from_secs(1));
        assert!(gw.running.is_none(), "not before the pause is over");
        gw.step(crashed + FIRST_PAUSE);
        assert_ne!(pid(&gw), first, "started again");

        gw.act(Action::Ack, crashed + FIRST_PAUSE);
        assert!(!gw.facts().contains_key("crash"));
        assert!(!dir.path().join("ssgw/crash/pending").exists(), "the record stays, the notice goes");
        assert!(kept.join("reason.txt").exists());
        gw.halt();
    }

    #[test]
    fn three_crashes_within_ten_minutes_stop_the_tries_until_try_again_or_the_next_boot() {
        let dir = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\necho 'ERROR no luck' >&2\nexit 3\n";
        let (mut gw, told) = gateway(dir.path(), script);
        let mut now = Instant::now();
        gw.on(now);
        for pause in [2, 4, 0] {
            gw.step(now);
            exited(&gw);
            now += Duration::from_secs(1);
            gw.step(now);
            assert!(gw.running.is_none());
            now += Duration::from_secs(pause);
        }
        let told = told.lock().clone();
        let titles: Vec<&str> = told.iter().map(|t| t.1.as_str()).collect();
        assert_eq!(titles, [CRASHED, CRASHED, STOPPED], "one message each, the last one saying it stopped");
        assert_eq!(told[1].2, "LumaTV's gateway on the TV exited with status 3 after 1 s. LG's gateway is serving Home Assistant; \
                               LumaTV's starts again in 4 s.");
        assert_eq!(told[2].2, format!("LumaTV's gateway on the TV exited with status 3 after 1 s: 3 crashes within 10 min. {LG_UNTIL}"));
        assert_eq!(gw.facts()["status"], "Stopped after 3 crashes within 10 min; LG's gateway is serving");
        assert_eq!(gw.facts()["gaveup"], "3 crashes within 10 min");
        assert_eq!(read(&record(dir.path()).join("reason.txt")).lines().nth(1), Some("kind: gave-up"));
        gw.step(now + Duration::from_secs(3600));
        assert!(gw.running.is_none(), "no more tries");

        // the helper again in the same boot, and a boot later
        let (mut again, _) = gateway(dir.path(), SLEEPS);
        again.on(now);
        again.step(now);
        assert!(again.running.is_none(), "still stopped");
        std::fs::write(dir.path().join("boot_id"), "boot-2\n").unwrap();
        let (mut booted, _) = gateway(dir.path(), SLEEPS);
        booted.on(now);
        booted.step(now);
        assert!(booted.running.is_some(), "a new boot tries again");
        booted.halt();

        again.act(Action::Retry, now);
        again.step(now);
        assert!(again.running.is_some(), "Try again starts it");
        assert!(!dir.path().join("ssgw/crash/gave-up").exists());
        again.halt();
    }

    #[test]
    fn a_gateway_that_cannot_serve_this_tv_stops_the_tries_at_once_and_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let script = "#!/bin/sh\necho 'ERROR /usr/bin/nsl is not the one this gateway was made for' >&2\nexit 78\n";
        let (mut gw, told) = gateway(dir.path(), script);
        let now = Instant::now();
        gw.on(now);
        gw.step(now);
        exited(&gw);
        gw.step(now);
        let told = told.lock().clone();
        assert_eq!(told.len(), 1);
        assert_eq!(
            told[0].2,
            format!("LumaTV's gateway on the TV says it cannot serve this TV (/usr/bin/nsl is not the one this gateway was made for). {LG_UNTIL}")
        );
        assert_eq!(gw.facts()["status"], "Stopped after it cannot serve this TV; LG's gateway is serving");
        assert_eq!(read(&record(dir.path()).join("reason.txt")).lines().nth(1), Some("kind: unusable"));
    }

    #[test]
    fn a_gateway_that_stops_answering_is_killed_after_three_missed_checks() {
        let dir = tempfile::tempdir().unwrap();
        let (mut gw, told) = gateway(dir.path(), SLEEPS);
        let t0 = Instant::now();
        gw.on(t0);
        gw.step(t0);
        let hung = pid(&gw);
        gw.step(t0 + TICK);
        gw.step(t0 + TICK * 2);
        assert_eq!(gw.facts()["status"], "Not answering (2 of 3 checks missed); LG's gateway takes new connections");
        gw.step(t0 + TICK * 3);
        assert!(gw.running.is_none() && gone(hung), "killed");
        let told = told.lock().clone();
        assert_eq!(told.len(), 1);
        assert!(told[0].2.starts_with("LumaTV's gateway on the TV stopped answering its health checks for 30 s after 30 s."), "{told:?}");
    }

    #[test]
    fn switched_off_the_gateway_stops_and_port_3001_goes_back_to_lgs_entry_without_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let (mut gw, told) = gateway(dir.path(), SLEEPS);
        let now = Instant::now();
        gw.on(now);
        gw.step(now);
        let running = pid(&gw);
        gw.off();
        assert!(gone(running));
        assert_eq!(read(&dir.path().join("calls")), "ssgw.sh on\nssgw.sh status\nssgw.sh off\n");
        assert!(told.lock().is_empty());
        assert!(!dir.path().join("ssgw/crash/pending").exists());
    }

    #[test]
    fn the_run_directory_is_the_gateways_own_whatever_was_there() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("run");
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &run).unwrap();
        run_dir(&run, None).unwrap();
        let m = std::fs::symlink_metadata(&run).unwrap();
        assert!(m.is_dir() && m.mode() & 0o7777 == 0o700, "a directory of its own");
        std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o777)).unwrap();
        run_dir(&run, None).unwrap();
        assert_eq!(std::fs::symlink_metadata(&run).unwrap().mode() & 0o7777, 0o700);
    }

    #[test]
    fn the_phone_is_told_through_home_assistants_notify_service() {
        let dir = tempfile::tempdir().unwrap();
        let ha = ha_link::fake::FakeHa::start("eyJhbGciOiJIUzI1NiJ9.ssgw-test-token.signature");
        let links = [dir.path().join("ha.json")];
        ha_store::save(&links[0], &ha.url(), "eyJhbGciOiJIUzI1NiJ9.ssgw-test-token.signature").unwrap();
        let config = Config::default();
        notify(&links, &config, AddressPolicy::TESTING, "notify.mobile_app_phone", CRASHED, "It crashed.").unwrap();
        let calls = ha.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!((calls[0].domain.as_str(), calls[0].service.as_str()), ("notify", "mobile_app_phone"));
        assert_eq!(Json::Object(calls[0].data.clone()), json!({"title": CRASHED, "message": "It crashed."}));
        assert!(notify(&links, &config, AddressPolicy::TESTING, "light.turn_on", CRASHED, "x").is_err());
    }

    /// What the broker answered, with the descriptor beside it.
    fn answer(end: &OwnedFd) -> (String, Option<OwnedFd>) {
        let mut bytes = [0u8; 16];
        let mut iov = libc::iovec { iov_base: bytes.as_mut_ptr().cast(), iov_len: bytes.len() };
        let mut control = [0u64; 4];
        // SAFETY: zero is a valid msghdr; `iov` and `control` outlive the call, and a header the kernel wrote lies within `control`.
        unsafe {
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = control.as_mut_ptr().cast();
            msg.msg_controllen = std::mem::size_of_val(&control) as _;
            let n = libc::recvmsg(end.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC);
            assert!(n > 0, "{}", std::io::Error::last_os_error());
            let header = libc::CMSG_FIRSTHDR(&msg);
            let fd = if header.is_null() {
                None
            } else {
                assert_eq!(((*header).cmsg_level, (*header).cmsg_type), (libc::SOL_SOCKET, libc::SCM_RIGHTS));
                Some(OwnedFd::from_raw_fd(libc::CMSG_DATA(header).cast::<libc::c_int>().read_unaligned()))
            };
            (String::from_utf8_lossy(&bytes[..n as usize]).into_owned(), fd)
        }
    }

    #[test]
    fn the_helper_connects_lgs_input_sockets_for_the_gateway_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let pointer = dir.path().join("netinput.pointer.sock");
        let absent = dir.path().join("netinput.gamepad.sock");
        let listener = UnixListener::bind(&pointer).unwrap();
        let (ours, theirs) = seqpacket_pair().unwrap();
        let allowed = vec![pointer.clone(), absent.clone()];
        let helper = std::thread::spawn(move || broker(&ours, &allowed));
        let ask = |path: &Path| {
            send(&theirs, path.as_os_str().as_bytes(), None).unwrap();
            answer(&theirs)
        };

        let (said, fd) = ask(&pointer);
        assert_eq!(said, "K");
        let mut connection = UnixStream::from(fd.expect("the connection"));
        // SAFETY: no memory is passed.
        assert_eq!(unsafe { libc::fcntl(connection.as_raw_fd(), libc::F_GETFL) } & libc::O_NONBLOCK, 0);
        connection.write_all(b"move").unwrap();
        let mut moved = [0; 4];
        listener.accept().unwrap().0.read_exact(&mut moved).unwrap();
        assert_eq!(&moved, b"move");

        let (said, fd) = ask(Path::new("/etc/passwd"));
        assert_eq!((said.as_str(), fd.is_none()), ("E13", true));
        let (said, fd) = ask(&absent);
        assert_eq!((said, fd.is_none()), (format!("E{}", libc::ENOENT), true));

        drop(theirs);
        helper.join().unwrap();
    }

    #[test]
    fn a_gateway_on_its_own_user_is_handed_the_brokers_end_and_one_as_root_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let seen = dir.path().join("seen");
        let script = format!(
            "#!/bin/sh\necho \"${{SSGW_BROKER_FD:-none}} $(readlink /proc/$$/fd/${{SSGW_BROKER_FD:-none}})\" >> '{}'\n",
            seen.display()
        );
        let (mut gw, _) = gateway(dir.path(), &script);
        gw.files.netinput = vec![dir.path().join("netinput.pointer.sock")];
        for as_root in [false, true] {
            gw.as_root = as_root;
            let mut run = gw.spawn(Instant::now()).unwrap();
            run.child.wait().unwrap();
        }
        let seen = read(&seen);
        let lines: Vec<&str> = seen.lines().collect();
        let (fd, what) = lines[0].split_once(' ').unwrap();
        assert!(fd.parse::<u32>().is_ok() && what.starts_with("socket:"), "{seen}");
        assert_eq!(lines[1].trim(), "none", "{seen}");
    }

    fn ctx(dir: &Path) -> Ctx {
        let hub = Arc::new(Hub::new(1));
        let paths = Paths::new(dir);
        let settings = Arc::new(SettingsStore::new(paths.settings_dir(), hub.clone()));
        settings.register_builtin().unwrap();
        Ctx {
            config: Arc::new(Config::default()),
            paths,
            hub,
            settings,
            stop: Stop::new(),
        }
    }

    fn soon(what: &str, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < end, "{what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn switched_on_through_the_base_the_page_shows_the_gateway_and_answers_its_actions() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let plugins = Plugins::new(&ctx);
        let told = Told::default();
        let (tx, rx) = mpsc::channel();
        let threads = [
            plugins.spawn(&ctx).unwrap(),
            spawn_with(&ctx, plugins.shared().clone(), files(dir.path(), SLEEPS), told_to(&told), tx, rx).unwrap(),
        ];
        let facts = || ctx.hub.get(&format!("plugin.{ID}.facts")).unwrap_or_default()["facts"].clone();
        let approve = || plugins.shared().approval_of(ID);
        soon("offered, off", || approve().is_some());
        plugins.switch(ID, true, approve().as_deref()).unwrap();
        soon("its page", || facts()["status"] == "Starting");
        assert_eq!(facts()["lgextras"], "Off: LumaTV keeps them from loading");
        plugins.shared().invoke_action(ID, RETRY, None).unwrap();
        plugins.shared().invoke_action(ID, ACK, None).unwrap();
        plugins.switch(ID, false, None).unwrap();
        soon("port 3001 given back", || read(&dir.path().join("calls")).ends_with("ssgw.sh off\n"));
        assert!(told.lock().is_empty());
        ctx.stop.request();
        for t in threads {
            t.join().unwrap();
        }
    }
}
