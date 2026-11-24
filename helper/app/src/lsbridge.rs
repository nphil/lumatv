//! One bus client for the helper's standing luna subscriptions.
//!
//! Each `luna-send -n 1000000` the helper keeps open for a state it follows is a process of its own, with its own copy of LG's
//! luna-service2 and glib state (0.4 MB of private memory apiece on the set, 13 of them). `lumatv-lsbridge` (`lsbridge/`, 32-bit like
//! LG's userland) is one process on the same bus that holds them all: the helper writes a line to its standard input to start or end
//! a call and reads the replies from its standard output (the protocol is at the top of `lumatv-lsbridge.c`).
//!
//! The helper carries the program (`build.rs`) and writes it, with the hub role it needs, under its data directory when it starts:
//! ls-hubd admits a program by its path and refuses one it has no role file for. Nothing here is required for a subscription to work.
//! A build without the program, a staging that fails, a bridge that does not say it is ready, and one that ends all leave each call to
//! the `luna-send` process it was made with; a bridge that ends is started again, not at once and not more than a few times in a
//! row. The bridge ends when the helper's end of its pipe closes, however the helper ends, so it cannot be left running.

use crate::luna::FeedOut;
use parking_lot::Mutex;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// The program's image; empty in a build for another machine or made without `lsbridge/build.sh`.
pub static IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/lumatv-lsbridge"));

/// Where the hub reads the roles and manifests of programs installed by hand (`/etc/luna-service2/ls-hubd.conf`:
/// `ManifestsVolatileDirectories`). A role file alone is not read: a manifest names it.
pub const LS2_DIR: &str = "/var/luna-service2-dev";
const NAME: &str = "lumatv-lsbridge";
const MANIFEST_ID: &str = "org.nphil.lumatv.lsbridge";

/// How long a bridge has to say it is ready.
const HELLO: Duration = Duration::from_secs(3);
/// How long after one that failed to start, or ended soon after, the next is tried; the bridge is given up on after `TRIES` in a row.
const RETRY: Duration = Duration::from_secs(60);
const TRIES: u32 = 3;
/// A bridge that ends before it has run this long counts as a failure to start.
const HEALTHY: Duration = Duration::from_secs(60);
/// The replies a subscription's reader may fall behind by before its subscription is ended (its reader re-subscribes).
const QUEUE: usize = 64;
/// The longest line the bridge may send.
const MAX_LINE: u64 = 4 * 1024 * 1024;

/// The first eight hex digits of the image's SHA-256, which the log names it by.
pub fn id(image: &[u8]) -> String {
    Sha256::digest(image).iter().take(4).map(|b| format!("{b:02x}")).collect()
}

// ------------------------------------------------------------------------------------------------------------------ staging

/// Where the program and its role are written.
pub struct Layout {
    /// Holds the program: only this user may write to it.
    pub dir: PathBuf,
    /// The hub's directory for roles and manifests installed by hand.
    pub ls2: PathBuf,
}

impl Layout {
    pub fn exe(&self) -> PathBuf {
        self.dir.join(NAME)
    }

    fn role(&self) -> PathBuf {
        self.ls2.join("roles.d").join("lumatv-lsbridge.json")
    }

    fn manifest(&self) -> PathBuf {
        self.ls2.join("manifests.d").join(format!("{MANIFEST_ID}.json"))
    }
}

/// What staging did.
pub struct Staged {
    pub exe: PathBuf,
    /// The hub's files were written (or changed): it has to be asked to read them.
    pub hub_files_changed: bool,
}

/// The role: matched by the program's path, with the one anonymous name (what `luna-send` registers) and no restriction on what it may
/// call: it calls what the helper's modules call through `luna-send`, with the permissions the hub gives that name.
fn role_text(exe: &Path) -> String {
    let role = json!({
        "exeName": exe,
        "type": "regular",
        "allowedNames": [""],
        "permissions": [{ "service": "", "inbound": [], "outbound": ["*"] }],
    });
    format!("{role:#}\n")
}

fn manifest_text(role: &Path) -> String {
    let manifest = json!({ "id": MANIFEST_ID, "version": "1.0.0", "roleFiles": [role] });
    format!("{manifest:#}\n")
}

/// Writes `bytes` to `path` through a file beside it that is renamed onto it, unless the file there is already exactly that. Answers
/// whether it wrote.
fn put(path: &Path, bytes: &[u8], mode: u32) -> io::Result<bool> {
    if fs::read(path).is_ok_and(|have| have == bytes) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".new");
    let fresh = path.with_file_name(name);
    let _ = fs::remove_file(&fresh);
    // written beside it and renamed over it, so that a copy a running bridge executes is never opened for writing
    let mut file = OpenOptions::new().write(true).create_new(true).mode(mode).open(&fresh)?;
    file.write_all(bytes)?;
    drop(file);
    fs::rename(&fresh, path)?;
    Ok(true)
}

/// Writes the program, its role and the manifest that names the role, where they are not already exactly these.
pub fn install(layout: &Layout, image: &[u8]) -> io::Result<Staged> {
    if image.is_empty() {
        return Err(io::Error::new(io::ErrorKind::Unsupported, "this build of the helper has no bus bridge in it"));
    }
    DirBuilder::new().recursive(true).mode(0o755).create(&layout.dir)?;
    // A directory another user could write to would let that user's file be run in the program's place, with the role.
    let own = fs::symlink_metadata(&layout.dir)?;
    // SAFETY: geteuid has no arguments and cannot fail.
    if !own.is_dir() || own.uid() != unsafe { libc::geteuid() } || own.mode() & 0o022 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is not this user's own directory", layout.dir.display()),
        ));
    }
    let exe = layout.exe();
    put(&exe, image, 0o755)?;
    let role = layout.role();
    let wrote_role = put(&role, role_text(&exe).as_bytes(), 0o644)?;
    let wrote_manifest = put(&layout.manifest(), manifest_text(&role).as_bytes(), 0o644)?;
    Ok(Staged { exe, hub_files_changed: wrote_role || wrote_manifest })
}

/// Asks the hub to read its role and manifest directories again (`ls-control`, which is on the set), so that the role is there for the
/// bridge's first start. A hub that cannot be asked reads them when it next starts.
pub fn rescan() {
    for what in ["scan-volatile-dirs", "scan-services"] {
        let Ok(mut child) = Command::new("ls-control").arg(what).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        else {
            crate::log_warn!("lsbridge: no ls-control: the hub reads the bridge's role when it next starts");
            return;
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        crate::log_warn!("lsbridge: ls-control {what} {status}");
                    }
                    break;
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    crate::log_warn!("lsbridge: ls-control {what} did not finish");
                    break;
                }
            }
        }
    }
}

// ----------------------------------------------------------------------------------------------------------------- the bridge

/// What decides when a bridge is started and given up on; a test shortens it.
#[derive(Clone, Copy)]
struct Timing {
    hello: Duration,
    retry: Duration,
    healthy: Duration,
}

const TIMING: Timing = Timing { hello: HELLO, retry: RETRY, healthy: HEALTHY };

enum State {
    /// Not in use: no program, or ended for good.
    Off,
    On {
        /// The running bridge, if there is one.
        conn: Option<Arc<Conn>>,
        /// Failed starts (or early ends) in a row.
        failures: u32,
        /// Not before this time.
        retry_at: Option<Instant>,
    },
}

/// The bridge the helper's subscriptions go through: its program, and the connection to the running one.
pub struct Bridge {
    program: PathBuf,
    /// The `luna-send` this stands in for: a subscription made with another program is not the bridge's.
    replaces: String,
    timing: Timing,
    state: Mutex<State>,
}

impl Bridge {
    pub fn new(program: PathBuf, replaces: &str) -> Self {
        Self::with_timing(program, replaces, TIMING)
    }

    fn with_timing(program: PathBuf, replaces: &str, timing: Timing) -> Self {
        Self {
            program,
            replaces: replaces.to_owned(),
            timing,
            state: Mutex::new(State::On { conn: None, failures: 0, retry_at: None }),
        }
    }

    /// Starts `uri` with `payload` as a subscription, starting the bridge if it is not running. `None` leaves the call to a `luna-send`
    /// of its own: the bridge is off, was given up on, is waiting to be tried again, or does not take this call.
    pub fn subscribe(&self, uri: &str, payload: &str) -> Option<Sub> {
        // a call is one line
        if uri.is_empty() || uri.contains(char::is_whitespace) || payload.contains(['\n', '\r']) {
            return None;
        }
        let mut state = self.state.lock();
        let State::On { conn, failures, retry_at } = &mut *state else { return None };
        if conn.as_ref().is_some_and(|c| !c.alive.load(Ordering::Acquire)) {
            if conn.take().is_some_and(|c| c.started.elapsed() < self.timing.healthy) {
                *failures += 1;
                *retry_at = Some(Instant::now() + self.timing.retry);
            }
            if *failures >= TRIES {
                crate::log_warn!("lsbridge: given up on after {TRIES} failures: subscriptions use luna-send");
                *state = State::Off;
                return None;
            }
        }
        if conn.is_none() {
            if retry_at.is_some_and(|at| Instant::now() < at) {
                return None;
            }
            match Conn::start(&self.program, self.timing.hello) {
                Ok(started) => {
                    crate::log_info!("lsbridge: started (pid {})", started.pid);
                    *conn = Some(started);
                    *retry_at = None;
                }
                Err(why) => {
                    *failures += 1;
                    *retry_at = Some(Instant::now() + self.timing.retry);
                    crate::log_warn!("lsbridge: not started ({why}): subscriptions use luna-send");
                    if *failures >= TRIES {
                        *state = State::Off;
                    }
                    return None;
                }
            }
        }
        conn.as_ref()?.subscribe(uri, payload)
    }

    /// Ends the bridge (and with it every subscription) and refuses any more: the helper is stopping.
    pub fn shutdown(&self) {
        let conn = match std::mem::replace(&mut *self.state.lock(), State::Off) {
            State::On { conn, .. } => conn,
            State::Off => None,
        };
        if let Some(conn) = conn {
            conn.close();
        }
    }
}

/// A running bridge: the child, the way in, and the subscriptions its replies are sorted to.
struct Conn {
    pid: u32,
    started: Instant,
    child: Mutex<Child>,
    /// `None` once closed: the bridge sees the end of its input and ends.
    stdin: Mutex<Option<ChildStdin>>,
    /// The subscriptions that are open, by the number the bridge knows them by.
    subs: Mutex<HashMap<u64, SyncSender<Vec<u8>>>>,
    next: AtomicU64,
    alive: AtomicBool,
}

impl Conn {
    /// Starts the bridge and waits for it to say it is ready.
    fn start(program: &Path, hello: Duration) -> Result<Arc<Self>, String> {
        let mut child = Command::new(program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot run {}: {e}", program.display()))?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("no pipes".into());
        };
        let conn = Arc::new(Self {
            pid: child.id(),
            started: Instant::now(),
            child: Mutex::new(child),
            stdin: Mutex::new(Some(stdin)),
            subs: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            alive: AtomicBool::new(true),
        });
        let (ready_tx, ready_rx) = mpsc::channel();
        let reader = {
            let conn = conn.clone();
            thread::Builder::new().name("lsbridge".into()).stack_size(128 * 1024).spawn(move || conn.read_loop(stdout, &ready_tx))
        };
        if let Err(e) = reader {
            conn.close();
            return Err(format!("no thread: {e}"));
        }
        match ready_rx.recv_timeout(hello) {
            Ok(Ok(())) => Ok(conn),
            Ok(Err(why)) => {
                conn.close();
                Err(why)
            }
            Err(_) => {
                conn.close();
                Err("it did not say it was ready".into())
            }
        }
    }

    fn subscribe(self: &Arc<Self>, uri: &str, payload: &str) -> Option<Sub> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::sync_channel(QUEUE);
        self.subs.lock().insert(id, tx);
        if self.send(&format!("S {id} {uri} {payload}\n")).is_err() {
            self.subs.lock().remove(&id);
            return None;
        }
        Some(Sub { conn: self.clone(), id, rx: Some(rx) })
    }

    fn send(&self, line: &str) -> io::Result<()> {
        match self.stdin.lock().as_mut() {
            Some(stdin) => stdin.write_all(line.as_bytes()),
            None => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }

    /// Ends one subscription: its reader's input ends and the bridge is told to drop the call.
    fn end(&self, id: u64) {
        let was_open = self.subs.lock().remove(&id).is_some();
        if was_open {
            let _ = self.send(&format!("C {id}\n"));
        }
    }

    /// Closes the bridge's input and waits for it to end, then makes sure of it.
    fn close(&self) {
        self.stdin.lock().take();
        let mut child = self.child.lock();
        let deadline = Instant::now() + Duration::from_secs(1);
        while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Sorts the bridge's lines: until its output ends, which ends every subscription.
    fn read_loop(&self, out: ChildStdout, ready: &mpsc::Sender<Result<(), String>>) {
        let mut out = BufReader::with_capacity(8 * 1024, out);
        let mut line = Vec::new();
        loop {
            line.clear();
            match out.by_ref().take(MAX_LINE).read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) if line.last() != Some(&b'\n') => break,
                Ok(_) => {}
            }
            line.pop();
            self.dispatch(&line, ready);
        }
        self.alive.store(false, Ordering::Release);
        let _ = ready.send(Err("it ended".into()));
        self.subs.lock().clear();
        let _ = self.child.lock().wait();
    }

    fn dispatch(&self, line: &[u8], ready: &mpsc::Sender<Result<(), String>>) {
        match line {
            [b'H', b' ', _version @ ..] => {
                let _ = ready.send(Ok(()));
            }
            [b'X', b' ', why @ ..] => {
                let why = String::from_utf8_lossy(why).into_owned();
                crate::log_warn!("lsbridge: {why}");
                let _ = ready.send(Err(why));
            }
            [kind @ (b'R' | b'E'), b' ', rest @ ..] => {
                let Some((id, text)) = split_id(rest) else { return };
                if *kind == b'E' {
                    crate::log_debug!("lsbridge: call {id} ended: {}", String::from_utf8_lossy(text));
                    self.subs.lock().remove(&id);
                    return;
                }
                let mut reply = Vec::with_capacity(text.len() + 1);
                reply.extend_from_slice(text);
                reply.push(b'\n');
                let outcome = self.subs.lock().get(&id).map(|tx| tx.try_send(reply));
                match outcome {
                    None | Some(Ok(())) => {}
                    Some(Err(TrySendError::Full(_))) => {
                        crate::log_warn!("lsbridge: the reader of call {id} fell {QUEUE} replies behind: the call is ended");
                        self.end(id);
                    }
                    Some(Err(TrySendError::Disconnected(_))) => self.end(id),
                }
            }
            _ => {}
        }
    }
}

/// The number at the start of `rest`, and the text after the space that follows it.
fn split_id(rest: &[u8]) -> Option<(u64, &[u8])> {
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    let id = std::str::from_utf8(&rest[..digits]).ok()?.parse().ok()?;
    match rest.get(digits) {
        Some(b' ') => Some((id, &rest[digits + 1..])),
        None => Some((id, &[])),
        Some(_) => None,
    }
}

/// One subscription held by the bridge.
pub struct Sub {
    conn: Arc<Conn>,
    id: u64,
    rx: Option<Receiver<Vec<u8>>>,
}

impl Sub {
    /// The replies, once: a second call has nothing.
    pub fn output(&mut self) -> Option<FeedOut> {
        self.rx.take().map(|rx| Box::new(Replies { rx, line: Vec::new(), at: 0 }) as FeedOut)
    }

    /// Whether the subscription is over: it was ended, or the bridge ended it or ended itself.
    pub fn has_ended(&self) -> bool {
        !self.conn.subs.lock().contains_key(&self.id)
    }

    /// Ends the subscription; its output ends with it.
    pub fn end(&mut self) {
        self.conn.end(self.id);
    }
}

impl Drop for Sub {
    fn drop(&mut self) {
        self.conn.end(self.id);
    }
}

/// The replies of one subscription as bytes, a line to a reply, until it ends.
struct Replies {
    rx: Receiver<Vec<u8>>,
    line: Vec<u8>,
    at: usize,
}

impl Read for Replies {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.at == self.line.len() {
            match self.rx.recv() {
                Ok(line) => {
                    self.line = line;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.line.len() - self.at);
        buf[..n].copy_from_slice(&self.line[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

// -------------------------------------------------------------------------------------------------------------------- global

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

/// Makes the bridge the way subscriptions made with `luna_send` are held, if it can be staged. Called once, by the helper's own start.
pub fn enable(data_dir: &Path, luna_send: &str) {
    let layout = Layout { dir: data_dir.join("lsbridge"), ls2: PathBuf::from(LS2_DIR) };
    match install(&layout, IMAGE) {
        Ok(staged) => {
            if staged.hub_files_changed {
                rescan();
            }
            crate::log_info!("lsbridge: {} ({}) serves the luna subscriptions", staged.exe.display(), id(IMAGE));
            let _ = BRIDGE.set(Bridge::new(staged.exe, luna_send));
        }
        Err(e) if e.kind() == io::ErrorKind::Unsupported => {}
        Err(e) => crate::log_warn!("lsbridge: not staged ({e}): subscriptions use luna-send"),
    }
}

/// A subscription held by the bridge, or `None` when it is not on or does not take this one.
pub fn subscribe(program: &str, uri: &str, payload: &str) -> Option<Sub> {
    let bridge = BRIDGE.get().filter(|b| b.replaces == program)?;
    bridge.subscribe(uri, payload)
}

/// Ends the bridge: the helper is stopping.
pub fn shutdown() {
    if let Some(bridge) = BRIDGE.get() {
        bridge.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    const FAST: Timing = Timing { hello: Duration::from_millis(400), retry: Duration::from_secs(3600), healthy: Duration::ZERO };

    /// A bridge as a shell script. `log` is a file it writes what it is sent to; the replies are `body`'s.
    fn fake(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("fake-bridge");
        crate::executable::write_executable(&path, &format!("#!/bin/sh\n{body}\n"));
        path
    }

    fn lines(out: impl Read, n: usize) -> Vec<String> {
        let mut reader = BufReader::new(out);
        let mut got = Vec::new();
        for _ in 0..n {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            got.push(line.trim_end().to_owned());
        }
        got
    }

    /// Answers every `S id uri payload` with two replies and records what it is sent.
    const ECHO: &str = r#"
echo "$$" >> "$0.pids"
echo "H 1"
while read -r cmd id rest; do
  echo "$cmd $id $rest" >> "$0.log"
  case "$cmd" in
    S) uri=${rest%% *}
       echo "R $id {\"uri\":\"$uri\",\"n\":1}"
       echo "R $id {\"uri\":\"$uri\",\"n\":2}" ;;
  esac
done"#;

    #[test]
    fn one_process_serves_every_subscription_and_each_gets_its_own_replies() {
        let dir = tempfile::tempdir().unwrap();
        let program = fake(dir.path(), ECHO);
        let bridge = Bridge::with_timing(program.clone(), "luna-send", FAST);
        let mut a = bridge.subscribe("luna://a/x", r#"{"subscribe":true}"#).unwrap();
        let mut b = bridge.subscribe("luna://b/y", "{}").unwrap();
        assert_eq!(lines(a.output().unwrap(), 2), [r#"{"uri":"luna://a/x","n":1}"#, r#"{"uri":"luna://a/x","n":2}"#]);
        assert_eq!(lines(b.output().unwrap(), 2), [r#"{"uri":"luna://b/y","n":1}"#, r#"{"uri":"luna://b/y","n":2}"#]);
        assert!(a.output().is_none(), "the replies are handed out once");
        bridge.shutdown();
        let pids = fs::read_to_string(format!("{}.pids", program.display())).unwrap();
        assert_eq!(pids.lines().count(), 1, "one process was started");
        let log = fs::read_to_string(format!("{}.log", program.display())).unwrap();
        assert!(log.contains(r#"S 1 luna://a/x {"subscribe":true}"#) && log.contains("S 2 luna://b/y {}"), "{log}");
    }

    #[test]
    fn ending_a_subscription_ends_its_output_tells_the_bridge_and_leaves_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let program = fake(dir.path(), ECHO);
        let bridge = Bridge::with_timing(program.clone(), "luna-send", FAST);
        let mut a = bridge.subscribe("luna://a/x", "{}").unwrap();
        let mut b = bridge.subscribe("luna://b/y", "{}").unwrap();
        let (mut out_a, mut out_b) = (a.output().unwrap(), b.output().unwrap());
        assert_eq!(lines(&mut out_a, 2).len(), 2, "both replies came");
        assert!(!a.has_ended());
        a.end();
        assert!(a.has_ended() && !b.has_ended());
        assert!(lines(&mut out_a, 5).is_empty(), "the output ends with the subscription");
        assert_eq!(lines(&mut out_b, 2).len(), 2);
        drop(b);
        bridge.shutdown();
        let log = fs::read_to_string(format!("{}.log", program.display())).unwrap();
        assert!(log.lines().any(|l| l == "C 1 "), "{log}");
        assert!(log.lines().any(|l| l == "C 2 "), "a dropped subscription is ended too: {log}");
    }

    #[test]
    fn a_bridge_that_ends_ends_every_subscription_and_is_not_started_again_at_once() {
        let dir = tempfile::tempdir().unwrap();
        // answers the first call, then ends
        let program = fake(dir.path(), "echo \"H 1\"\nread -r cmd id rest\necho \"R $id {\\\"ok\\\":true}\"");
        let bridge = Bridge::with_timing(program, "luna-send", Timing { healthy: Duration::from_secs(3600), ..FAST });
        let mut a = bridge.subscribe("luna://a/x", "{}").unwrap();
        assert_eq!(lines(a.output().unwrap(), 5), [r#"{"ok":true}"#], "the reply, then the end");
        assert!(a.has_ended());
        // it ended at once, which is a failure to start: the next call is left to luna-send, and so are those after it
        assert!(bridge.subscribe("luna://b/y", "{}").is_none());
        assert!(bridge.subscribe("luna://b/y", "{}").is_none());
    }

    #[test]
    fn a_bridge_that_never_says_it_is_ready_or_cannot_register_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let silent = Bridge::with_timing(fake(dir.path(), "exec sleep 30"), "luna-send", Timing { hello: Duration::from_millis(50), ..FAST });
        assert!(silent.subscribe("luna://a/x", "{}").is_none());
        let refused = Bridge::with_timing(fake(dir.path(), "echo 'X cannot register: no role'"), "luna-send", FAST);
        assert!(refused.subscribe("luna://a/x", "{}").is_none());
        let missing = Bridge::with_timing(dir.path().join("none"), "luna-send", FAST);
        assert!(missing.subscribe("luna://a/x", "{}").is_none());
    }

    #[test]
    fn a_reader_that_falls_behind_has_its_subscription_ended_not_queued_without_end() {
        let dir = tempfile::tempdir().unwrap();
        // two calls; the first is answered more often than the queue holds, then the second is answered: the lines are sorted in order
        let program = fake(
            dir.path(),
            r#"echo "H 1"
read -r c1 id1 rest1
read -r c2 id2 rest2
i=0
while [ $i -lt 100 ]; do echo "R $id1 {\"n\":$i}"; i=$((i+1)); done
echo "R $id2 {\"done\":true}"
cat > /dev/null"#,
        );
        let bridge = Bridge::with_timing(program, "luna-send", FAST);
        let mut slow = bridge.subscribe("luna://a/x", "{}").unwrap();
        let mut done = bridge.subscribe("luna://b/y", "{}").unwrap();
        assert_eq!(lines(done.output().unwrap(), 1), [r#"{"done":true}"#]);
        assert!(slow.has_ended(), "it was ended when its queue was full");
        let got = lines(slow.output().unwrap(), 200);
        assert_eq!(got.len(), QUEUE, "what was queued is kept, in order");
        assert_eq!(got[0], r#"{"n":0}"#);
        bridge.shutdown();
    }

    #[test]
    fn a_call_the_bridge_cannot_make_ends_that_subscription_alone() {
        let dir = tempfile::tempdir().unwrap();
        let program = fake(
            dir.path(),
            r#"echo "H 1"
read -r c1 id1 rest1
read -r c2 id2 rest2
echo "E $id1 refused"
echo "R $id2 {\"ok\":true}"
cat > /dev/null"#,
        );
        let bridge = Bridge::with_timing(program, "luna-send", FAST);
        let mut bad = bridge.subscribe("luna://a/x", "{}").unwrap();
        let mut good = bridge.subscribe("luna://b/y", "{}").unwrap();
        assert_eq!(lines(good.output().unwrap(), 1), [r#"{"ok":true}"#]);
        assert!(bad.has_ended());
        assert!(lines(bad.output().unwrap(), 1).is_empty());
        bridge.shutdown();
    }

    #[test]
    fn a_call_that_is_not_one_line_is_left_to_luna_send() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = Bridge::with_timing(fake(dir.path(), ECHO), "luna-send", FAST);
        assert!(bridge.subscribe("luna://a/x", "{\n}").is_none());
        assert!(bridge.subscribe("luna://a /x", "{}").is_none());
        assert!(bridge.subscribe("", "{}").is_none());
        bridge.shutdown();
        assert!(bridge.subscribe("luna://a/x", "{}").is_none(), "none after it is shut down");
    }

    #[test]
    fn the_program_the_role_and_the_manifest_are_written_once_and_again_when_they_differ() {
        let base = tempfile::tempdir().unwrap();
        let layout = Layout { dir: base.path().join("data/lsbridge"), ls2: base.path().join("ls2") };
        let first = install(&layout, b"first").unwrap();
        assert!(first.hub_files_changed);
        assert_eq!(fs::read(&first.exe).unwrap(), b"first");
        let role: serde_json::Value = serde_json::from_slice(&fs::read(layout.role()).unwrap()).unwrap();
        assert_eq!(role["exeName"], first.exe.to_str().unwrap());
        assert_eq!(role["allowedNames"], json!([""]));
        let manifest: serde_json::Value = serde_json::from_slice(&fs::read(layout.manifest()).unwrap()).unwrap();
        assert_eq!(manifest["roleFiles"], json!([layout.role().to_str().unwrap()]));
        let again = install(&layout, b"first").unwrap();
        assert!(!again.hub_files_changed, "the hub is not asked to read what it has");
        let at = fs::metadata(&first.exe).unwrap().ino();
        install(&layout, b"first").unwrap();
        assert_eq!(fs::metadata(&first.exe).unwrap().ino(), at, "the same file is kept");
        install(&layout, b"second").unwrap();
        assert_eq!(fs::read(&first.exe).unwrap(), b"second");
        assert!(install(&layout, b"").is_err_and(|e| e.kind() == io::ErrorKind::Unsupported));
    }

    #[test]
    fn a_directory_other_users_can_write_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().unwrap();
        let open = base.path().join("open");
        fs::create_dir(&open).unwrap();
        fs::set_permissions(&open, fs::Permissions::from_mode(0o777)).unwrap();
        let layout = Layout { dir: open, ls2: base.path().join("ls2") };
        assert!(install(&layout, b"x").is_err_and(|e| e.kind() == io::ErrorKind::PermissionDenied));
        assert!(!layout.ls2.exists(), "nothing is written for the hub");
    }

    #[test]
    fn a_line_is_split_into_its_number_and_text() {
        assert_eq!(split_id(b"12 {\"a\":1}"), Some((12, &b"{\"a\":1}"[..])));
        assert_eq!(split_id(b"7"), Some((7, &b""[..])));
        assert_eq!(split_id(b"x 1"), None);
        assert_eq!(split_id(b"7x"), None);
        assert_eq!(split_id(b""), None);
    }

    #[test]
    fn the_image_is_named_by_the_start_of_its_hash() {
        assert_eq!(id(b"abc"), "ba7816bf");
    }
}
