//! Calls on the TV's bus through `luna-send`: one process per call or subscription, each reply one line on its standard
//! output. Subscriptions every client would make alike are shared by one process, and a client that joins one late gets its last
//! reply at once.

use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::os::fd::{AsRawFd, RawFd};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// One-shot calls running at once; more wait their turn.
pub const MAX_CALLS: usize = 8;
/// `luna-send` processes at once, calls and subscriptions together (the gateway runs under a ceiling of 64 processes).
pub const MAX_PROCS: usize = 48;
const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Replies longer than this are not JSON this gateway passes on.
const LINE_MAX: usize = 16 * 1024 * 1024;

/// The subscriptions that answer every caller alike.
const SHAREABLE: &[&str] = &[
    "luna://com.webos.service.tvpower/power/getPowerState",
    "luna://com.webos.service.audio/master/getVolume",
    "luna://com.webos.settingsservice/getSystemSettings",
    "luna://com.webos.applicationManager/getForegroundAppInfo",
    "luna://com.webos.media/getForegroundAppInfo",
    "luna://com.webos.service.eim/getAllInputStatus",
];

/// Who a reply is for: a session (0 is the gateway itself), one of its operations, and which of that operation's calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Owner {
    pub sess: u32,
    pub op: u32,
    pub tag: u32,
}

pub enum Event {
    Reply(Owner, Rc<Value>),
    /// The call or subscription ended without (another) reply: the process exited, timed out or could not start.
    End(Owner),
}

struct Proc {
    child: Child,
    out: ChildStdout,
    buf: Vec<u8>,
    owners: Vec<Owner>,
    sub: bool,
    key: Option<String>,
    last: Option<Rc<Value>>,
    deadline: Option<Instant>,
}

struct Queued {
    owner: Owner,
    uri: String,
    payload: String,
}

pub struct Luna {
    bin: PathBuf,
    procs: HashMap<u32, Proc>,
    next: u32,
    queue: VecDeque<Queued>,
    shared: HashMap<String, u32>,
    events: Vec<Event>,
    /// Ended processes not yet reaped.
    dying: Vec<(Child, Instant)>,
}

impl Luna {
    pub fn new(bin: PathBuf) -> Luna {
        Luna { bin, procs: HashMap::new(), next: 1, queue: VecDeque::new(), shared: HashMap::new(), events: Vec::new(), dying: Vec::new() }
    }

    fn calls_running(&self) -> usize {
        self.procs.values().filter(|p| !p.sub).count()
    }

    /// A one-shot call; its reply (or its end) comes back as an event.
    pub fn call(&mut self, owner: Owner, uri: &str, payload: &Value) {
        let payload = payload.to_string();
        if self.calls_running() >= MAX_CALLS || self.procs.len() >= MAX_PROCS {
            self.queue.push_back(Queued { owner, uri: uri.to_owned(), payload });
            return;
        }
        self.spawn(owner, uri, &payload, false, None);
    }

    /// A subscription; `false` when there is no room for another process (an `End` event follows).
    pub fn subscribe(&mut self, owner: Owner, uri: &str, payload: &Value) -> bool {
        let payload = payload.to_string();
        let key = SHAREABLE.contains(&uri).then(|| format!("{uri}\n{payload}"));
        if let Some(pid) = key.as_ref().and_then(|k| self.shared.get(k)) {
            let proc = self.procs.get_mut(pid).expect("shared process");
            proc.owners.push(owner);
            if let Some(last) = &proc.last {
                self.events.push(Event::Reply(owner, last.clone()));
            }
            return true;
        }
        if self.procs.len() >= MAX_PROCS {
            crate::warn!("no room for another luna-send ({MAX_PROCS} running); refusing a subscription to {uri}");
            self.events.push(Event::End(owner));
            return false;
        }
        self.spawn(owner, uri, &payload, true, key)
    }

    fn spawn(&mut self, owner: Owner, uri: &str, payload: &str, sub: bool, key: Option<String>) -> bool {
        let count = if sub { "1000000" } else { "1" };
        let spawned = Command::new(&self.bin)
            .args(["-n", count, uri, payload])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                crate::error!("could not run {}: {e}", self.bin.display());
                self.events.push(Event::End(owner));
                return false;
            }
        };
        let out = child.stdout.take().expect("piped stdout");
        set_nonblocking(out.as_raw_fd());
        let pid = self.next;
        self.next = self.next.wrapping_add(1).max(1);
        if let Some(k) = &key {
            self.shared.insert(k.clone(), pid);
        }
        let deadline = (!sub).then(|| Instant::now() + CALL_TIMEOUT);
        self.procs.insert(pid, Proc { child, out, buf: Vec::new(), owners: vec![owner], sub, key, last: None, deadline });
        true
    }

    /// Stops what `owner` asked for.
    pub fn cancel(&mut self, owner: Owner) {
        self.cancel_where(|o| *o == owner);
    }

    /// Stops every call and subscription of one operation.
    pub fn cancel_op(&mut self, sess: u32, op: u32) {
        self.cancel_where(|o| o.sess == sess && o.op == op);
    }

    /// Stops everything a session asked for.
    pub fn cancel_session(&mut self, sess: u32) {
        self.cancel_where(|o| o.sess == sess);
    }

    fn cancel_where(&mut self, f: impl Fn(&Owner) -> bool) {
        self.queue.retain(|q| !f(&q.owner));
        self.events.retain(|e| match e {
            Event::Reply(o, _) | Event::End(o) => !f(o),
        });
        let mut empty = Vec::new();
        for (pid, p) in &mut self.procs {
            p.owners.retain(|o| !f(o));
            if p.owners.is_empty() {
                empty.push(*pid);
            }
        }
        for pid in empty {
            self.finish(pid);
        }
        self.start_queued();
    }

    fn finish(&mut self, pid: u32) {
        if let Some(p) = self.procs.remove(&pid) {
            if let Some(k) = &p.key {
                self.shared.remove(k);
            }
            let mut child = p.child;
            // SAFETY: a signal to the gateway's own child, still unreaped, so the pid is its.
            unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
            if !matches!(child.try_wait(), Ok(Some(_))) {
                self.dying.push((child, Instant::now()));
            }
        }
    }

    fn start_queued(&mut self) {
        while self.calls_running() < MAX_CALLS && self.procs.len() < MAX_PROCS {
            let Some(q) = self.queue.pop_front() else { break };
            self.spawn(q.owner, &q.uri, &q.payload, false, None);
        }
    }

    pub fn fds(&self) -> impl Iterator<Item = (u32, RawFd)> + '_ {
        self.procs.iter().map(|(pid, p)| (*pid, p.out.as_raw_fd()))
    }

    /// Reads what a process wrote; replies become events.
    pub fn readable(&mut self, pid: u32) {
        let Some(p) = self.procs.get_mut(&pid) else { return };
        let mut chunk = [0u8; 16 * 1024];
        let mut eof = false;
        loop {
            match p.out.read(&mut chunk) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => p.buf.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => {
                    eof = true;
                    break;
                }
            }
        }
        let mut done = false;
        while let Some(nl) = p.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = p.buf.drain(..=nl).collect();
            let Ok(v) = serde_json::from_slice::<Value>(&line) else { continue };
            let v = Rc::new(v);
            for o in &p.owners {
                self.events.push(Event::Reply(*o, v.clone()));
            }
            if p.sub {
                p.last = Some(v);
            } else {
                done = true;
                break;
            }
        }
        if p.buf.len() > LINE_MAX {
            crate::warn!("a luna-send reply passed {LINE_MAX} bytes; ending that call");
            eof = true;
        }
        if done {
            self.finish(pid);
            self.start_queued();
        } else if eof {
            // the call ended without (another) reply
            for o in &p.owners {
                self.events.push(Event::End(*o));
            }
            self.finish(pid);
            self.start_queued();
        }
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        let reap = self.dying.first().map(|(_, at)| *at + Duration::from_millis(200));
        self.procs.values().filter_map(|p| p.deadline).chain(reap).min()
    }

    /// Ends calls that ran out of time and reaps ended processes.
    pub fn tick(&mut self, now: Instant) {
        let late: Vec<u32> = self.procs.iter().filter(|(_, p)| p.deadline.is_some_and(|d| d <= now)).map(|(pid, _)| *pid).collect();
        for pid in late {
            if let Some(p) = self.procs.get(&pid) {
                crate::warn!("a luna-send call gave no answer in {} s", CALL_TIMEOUT.as_secs());
                for o in &p.owners {
                    self.events.push(Event::End(*o));
                }
            }
            self.finish(pid);
        }
        self.start_queued();
        self.dying.retain_mut(|(child, since)| {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return false;
            }
            if now.duration_since(*since) > Duration::from_secs(2) {
                let _ = child.kill();
                return child.wait().is_err();
            }
            true
        });
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    #[cfg(test)]
    pub fn running(&self) -> usize {
        self.procs.len()
    }

    /// Ends every process (the gateway is stopping).
    pub fn stop_all(&mut self) {
        let pids: Vec<u32> = self.procs.keys().copied().collect();
        for pid in pids {
            self.finish(pid);
        }
        self.queue.clear();
        for (mut child, _) in self.dying.drain(..) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn set_nonblocking(fd: RawFd) {
    // SAFETY: fcntl on a descriptor the caller owns.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}

/// A blocking call used once at start (the TLS key), outside the event loop.
pub fn call_blocking(bin: &std::path::Path, uri: &str, payload: &str, timeout: Duration) -> Result<Value, String> {
    let mut child = Command::new(bin)
        .args(["-n", "1", uri, payload])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run {}: {e}", bin.display()))?;
    let mut out = child.stdout.take().expect("piped stdout");
    let fd = out.as_raw_fd();
    let deadline = Instant::now() + timeout;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one pollfd that lives across the call.
        let n = unsafe { libc::poll(&mut pfd, 1, left.as_millis().min(i32::MAX as u128) as i32) };
        if n == 0 {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{uri} gave no answer in {} s", timeout.as_secs()));
        }
        match out.read(&mut chunk) {
            Ok(0) => break,
            Ok(k) => {
                buf.extend_from_slice(&chunk[..k]);
                if buf.contains(&b'\n') {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("reading {uri}: {e}")),
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let line = buf.split(|b| *b == b'\n').next().unwrap_or_default();
    serde_json::from_slice(line).map_err(|_| format!("{uri} did not answer with JSON"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;

    /// A `luna-send` stand-in: answers each URI with the lines in `<dir>/<uri with / and : as _>`, then (for a subscription)
    /// waits until killed.
    pub fn fake_bus(dir: &std::path::Path) -> PathBuf {
        let bin = dir.join("luna-send");
        let script = r#"#!/bin/sh
n=$2; uri=$3; body=$4
f="$(dirname "$0")/$(printf '%s' "$uri" | tr '/:' '__')"
printf '%s %s\n' "$uri" "$body" >> "$(dirname "$0")/calls.log"
[ -f "$f" ] && cat "$f"
[ "$n" = 1 ] || exec sleep 1000
"#;
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    pub fn answer(dir: &std::path::Path, uri: &str, lines: &[Value]) {
        let name = uri.replace(['/', ':'], "_");
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(dir.join(name), body).unwrap();
    }

    fn pump(luna: &mut Luna, want: usize) -> Vec<Event> {
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        while got.len() < want && Instant::now() < deadline {
            let mut fds: Vec<libc::pollfd> = luna.fds().map(|(_, fd)| libc::pollfd { fd, events: libc::POLLIN, revents: 0 }).collect();
            let pids: Vec<u32> = luna.fds().map(|(p, _)| p).collect();
            // SAFETY: the pollfds live across the call.
            unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 50) };
            for (i, f) in fds.iter().enumerate() {
                if f.revents != 0 {
                    luna.readable(pids[i]);
                }
            }
            got.extend(luna.take_events());
        }
        got
    }

    #[test]
    fn calls_answer_once_and_shared_subscriptions_replay_their_last_reply() {
        let dir = tempfile::tempdir().unwrap();
        let mut luna = Luna::new(fake_bus(dir.path()));
        answer(dir.path(), "luna://com.webos.service.audio/master/getVolume", &[json!({"returnValue": true, "volume": 7})]);
        let a = Owner { sess: 1, op: 1, tag: 0 };
        let b = Owner { sess: 2, op: 1, tag: 0 };
        luna.subscribe(a, "luna://com.webos.service.audio/master/getVolume", &json!({"subscribe": true}));
        let first = pump(&mut luna, 1);
        assert!(matches!(&first[..], [Event::Reply(o, v)] if *o == a && v["volume"] == 7));
        luna.subscribe(b, "luna://com.webos.service.audio/master/getVolume", &json!({"subscribe": true}));
        assert_eq!(luna.running(), 1, "the second client shares the first one's process");
        assert!(matches!(&luna.take_events()[..], [Event::Reply(o, v)] if *o == b && v["volume"] == 7));
        luna.cancel(a);
        assert_eq!(luna.running(), 1);
        luna.cancel_session(2);
        assert_eq!(luna.running(), 0);

        let c = Owner { sess: 3, op: 2, tag: 1 };
        luna.call(c, "luna://com.webos.service.none/x", &json!({}));
        assert!(matches!(&pump(&mut luna, 1)[..], [Event::End(o)] if *o == c), "a call with no reply ends");
        luna.stop_all();
    }
}
