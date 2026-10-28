//! The GeForce NOW app's pad reader, given a real-time slot: the one privilege the app's jail cannot give itself.
//!
//! The app's engine reads the controller on one thread per pad, blocked in `read()` on the evdev node. The jail has `RLIMIT_RTPRIO` 0
//! and no `CAP_SYS_NICE`, so the thread cannot raise itself; this module does it for it, root to thread, and for nothing else
//! (docs/research/gfn-input-result.md, "Priority"). Under load a normal-priority reader waits behind the decoder and the UI; at
//! `SCHED_FIFO` 10 it wakes at once, and pinned off CPU 0 it does not queue behind the OHCI interrupt that serves the receiver.
//!
//! What is refused, each with its own code (Principle 0: the grant is as narrow as the need):
//!
//! | Check | Refusal |
//! |---|---|
//! | the thread does not exist in that process | `not_found` |
//! | the thread's name is not `gfn-pad-event<N>` (the reader's own name) | `refused` |
//! | the process is root's | `refused` |
//! | neither the process nor an ancestor (6 levels) runs from `org.nphil.lumatv.gfn`'s directory | `refused` |
//! | four threads are boosted already | `refused` |
//!
//! The policy is `SCHED_FIFO` priority 10, with `SCHED_RESET_ON_FORK` so a child never inherits it; nothing else is asked or
//! accepted. The boost is a lease: it ends at once on `unboost`, when the thread is gone (the sweep notices a missing task, or a new
//! one under the same number by its start time), and [`LEASE`] after the last request: the app renews while the stream is in front,
//! so a hung or killed app cannot leave a thread elevated. The thread's own scheduling and affinity are put back, not guessed.
//! The kernel's `sched_rt_runtime_us` (95 %) stays the ceiling for a reader that spins.
//!
//! | Request | Answer |
//! |---|---|
//! | `POST /v1/gfn/pad-boost` `{pid, tid}` | `{boosted: {tid, priority, cpus, leaseSecs}}`, or `403 refused`, `404 not_found`, `500 sched_failed` |
//! | `POST /v1/gfn/pad-unboost` `{pid, tid}` | `{released: bool}` |
//! | `GET /v1/gfn/pad-boost` | what is boosted now |
use crate::API_VERSION;
use crate::http::{Request, Response, error_response};
use crate::timeutil::now_ms;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

/// The app whose process tree a boosted thread must belong to.
pub const APP_ID: &str = "org.nphil.lumatv.gfn";
/// The reader thread's name (`gfn-pad-event15`, the pad crate's `format!("gfn-pad-{node}")`); the kernel keeps 15 characters, so
/// `gfn-pad-event123` is `gfn-pad-event12`: the prefix is what is matched.
pub const READER_PREFIX: &str = "gfn-pad-event";
/// The real-time priority: low in FIFO's 1-99, above every ordinary thread.
pub const PRIORITY: i32 = 10;
/// How long a boost lives without a renewal.
pub const LEASE: Duration = Duration::from_secs(90);
/// Never more than the app's pads.
pub const MAX_BOOSTED: usize = 4;
/// Where the CPU affinity goes: off CPU 0 (the OHCI interrupt), on whichever of 1-3 are online (LG hot-unplugs 2 and 3).
const CPUS: [usize; 3] = [1, 2, 3];
const ANCESTOR_LEVELS: usize = 6;

pub const ROUTES: &[(&str, &str, &str)] = &[
    (
        "POST",
        "/v1/gfn/pad-boost",
        "the GeForce NOW app's pad reader thread {pid, tid}: SCHED_FIFO 10, off CPU 0, as a lease the app renews; anything else is refused",
    ),
    (
        "POST",
        "/v1/gfn/pad-unboost",
        "end the boost of a thread {pid, tid} and restore its scheduling",
    ),
    ("GET", "/v1/gfn/pad-boost", "the threads boosted now"),
];

/// A thread's scheduling as it was before the boost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// The policy with `SCHED_RESET_ON_FORK` if it was set, as `sched_getscheduler` returns it.
    pub policy: i32,
    pub priority: i32,
    pub cpus: Vec<usize>,
}

/// The scheduler calls; the host tests stand in for them.
pub trait Sched: Send + Sync {
    fn get(&self, tid: i32) -> io::Result<Saved>;
    /// Affinity (`cpus`, if any), then FIFO at `priority` with reset-on-fork.
    fn boost(&self, tid: i32, priority: i32, cpus: Option<&[usize]>) -> io::Result<()>;
    fn restore(&self, tid: i32, saved: &Saved) -> io::Result<()>;
}

pub struct RealSched;

fn cpu_set(cpus: &[usize]) -> libc::cpu_set_t {
    // SAFETY: cpu_set_t is plain bytes; CPU_SET writes within it for indexes below its size.
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    for &c in cpus {
        if c < 1024 {
            unsafe { libc::CPU_SET(c, &mut set) };
        }
    }
    set
}

fn set_affinity(tid: i32, cpus: &[usize]) -> io::Result<()> {
    let set = cpu_set(cpus);
    // SAFETY: a valid set and its size; the tid is any task of the system (Linux takes a thread id as a pid).
    if unsafe { libc::sched_setaffinity(tid, std::mem::size_of::<libc::cpu_set_t>(), &set) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn set_policy(tid: i32, policy: i32, priority: i32) -> io::Result<()> {
    let param = libc::sched_param {
        sched_priority: priority,
    };
    // SAFETY: a valid sched_param for the call's duration.
    if unsafe { libc::sched_setscheduler(tid, policy, &param) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

impl Sched for RealSched {
    fn get(&self, tid: i32) -> io::Result<Saved> {
        // SAFETY: plain queries into valid out-parameters.
        let policy = unsafe { libc::sched_getscheduler(tid) };
        if policy < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut param = libc::sched_param { sched_priority: 0 };
        if unsafe { libc::sched_getparam(tid, &mut param) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
        if unsafe { libc::sched_getaffinity(tid, std::mem::size_of::<libc::cpu_set_t>(), &mut set) }
            < 0
        {
            return Err(io::Error::last_os_error());
        }
        let cpus = (0..1024)
            .filter(|&c| unsafe { libc::CPU_ISSET(c, &set) })
            .collect();
        Ok(Saved {
            policy,
            priority: param.sched_priority,
            cpus,
        })
    }

    fn boost(&self, tid: i32, priority: i32, cpus: Option<&[usize]>) -> io::Result<()> {
        let before = if cpus.is_some() {
            Some(self.get(tid)?)
        } else {
            None
        };
        if let Some(c) = cpus {
            set_affinity(tid, c)?;
        }
        if let Err(e) = set_policy(tid, libc::SCHED_FIFO | libc::SCHED_RESET_ON_FORK, priority) {
            if let Some(b) = before {
                let _ = set_affinity(tid, &b.cpus);
            }
            return Err(e);
        }
        Ok(())
    }

    fn restore(&self, tid: i32, saved: &Saved) -> io::Result<()> {
        let policy = set_policy(tid, saved.policy, saved.priority);
        let affinity = if saved.cpus.is_empty() {
            Ok(())
        } else {
            set_affinity(tid, &saved.cpus)
        };
        policy.and(affinity)
    }
}

/// `/proc`, as far as the checks look: a directory the host tests fill with a made-up process table.
#[derive(Clone, Debug)]
pub struct Proc {
    root: PathBuf,
}

impl Proc {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn read(&self, parts: &[&str]) -> Option<String> {
        let mut p = self.root.clone();
        p.extend(parts);
        std::fs::read_to_string(p).ok()
    }

    fn comm(&self, pid: i32, tid: i32) -> Option<String> {
        self.read(&[&pid.to_string(), "task", &tid.to_string(), "comm"])
            .map(|s| s.trim_end().to_owned())
    }

    /// Fields of a `stat` line after the command (which may hold spaces and parentheses): state is the first.
    fn stat_fields(&self, parts: &[&str]) -> Option<Vec<String>> {
        let text = self.read(parts)?;
        let after = text.get(text.rfind(')')? + 1..)?;
        Some(after.split_whitespace().map(str::to_owned).collect())
    }

    /// The tick count at which the task started: with the number, a task's identity.
    fn started(&self, pid: i32, tid: i32) -> Option<String> {
        let f = self.stat_fields(&[&pid.to_string(), "task", &tid.to_string(), "stat"])?;
        f.get(19).cloned()
    }

    fn ppid(&self, pid: i32) -> Option<i32> {
        self.stat_fields(&[&pid.to_string(), "stat"])?
            .get(1)?
            .parse()
            .ok()
    }

    fn uid(&self, pid: i32) -> Option<u32> {
        let status = self.read(&[&pid.to_string(), "status"])?;
        status
            .lines()
            .find_map(|l| l.strip_prefix("Uid:"))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }

    /// Whether the process runs from the app's directory: its executable or one of its arguments has the app's id as a path
    /// component (the engine runs through the package's loader, so the engine's path is an argument; the app's own `main` is the exe).
    fn runs_app(&self, pid: i32) -> bool {
        let has_id = |s: &str| s.split('/').any(|c| c == APP_ID);
        let exe = std::fs::read_link(self.root.join(pid.to_string()).join("exe")).ok();
        if exe.is_some_and(|e| has_id(&e.to_string_lossy())) {
            return true;
        }
        self.read(&[&pid.to_string(), "cmdline"])
            .is_some_and(|c| c.split('\0').any(has_id))
    }

    fn in_app_tree(&self, pid: i32) -> bool {
        let mut p = pid;
        for _ in 0..ANCESTOR_LEVELS {
            if p <= 1 {
                return false;
            }
            if self.runs_app(p) {
                return true;
            }
            match self.ppid(p) {
                Some(parent) => p = parent,
                None => return false,
            }
        }
        false
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refusal {
    NotFound(String),
    Refused(String),
    Sched(String),
}

impl Refusal {
    fn response(&self) -> Response {
        match self {
            Refusal::NotFound(m) => error_response(404, "not_found", m),
            Refusal::Refused(m) => error_response(403, "refused", m),
            Refusal::Sched(m) => error_response(500, "sched_failed", m),
        }
    }
}

struct Entry {
    pid: i32,
    tid: i32,
    started: String,
    saved: Saved,
    lease_ends: Instant,
}

/// The boosted threads and the checks that admit them.
pub struct Boost {
    proc: Proc,
    sched: Box<dyn Sched>,
    /// `/sys/devices/system/cpu`: its `online` file says which of CPUs 1-3 exist right now.
    cpu_dir: PathBuf,
    entries: Mutex<Vec<Entry>>,
}

/// `0-1,3` as the kernel prints a CPU list.
fn parse_cpu_list(text: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in text.trim().split(',') {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) {
                    out.extend(a..=b.min(1023));
                }
            }
            None => out.extend(part.parse::<usize>()),
        }
    }
    out
}

impl Boost {
    pub fn new(
        proc_root: impl Into<PathBuf>,
        cpu_dir: impl Into<PathBuf>,
        sched: Box<dyn Sched>,
    ) -> Self {
        Self {
            proc: Proc::new(proc_root),
            sched,
            cpu_dir: cpu_dir.into(),
            entries: Mutex::new(Vec::new()),
        }
    }

    /// CPUs 1-3 that are online now; `None` when CPU 1 is not (a one-core set has nowhere to move to).
    fn target_cpus(&self) -> Option<Vec<usize>> {
        let online = parse_cpu_list(&std::fs::read_to_string(self.cpu_dir.join("online")).ok()?);
        let cpus: Vec<usize> = CPUS
            .iter()
            .copied()
            .filter(|c| online.contains(c))
            .collect();
        if cpus.contains(&1) { Some(cpus) } else { None }
    }

    /// Admit and boost a reader thread, or renew its lease. Every check is made again on a renewal.
    pub fn boost(&self, pid: i32, tid: i32, now: Instant) -> Result<Value, Refusal> {
        self.sweep(now);
        if pid <= 1 || tid <= 1 {
            return Err(Refusal::Refused("not a process of the app".into()));
        }
        let Some(comm) = self.proc.comm(pid, tid) else {
            return Err(Refusal::NotFound(format!(
                "process {pid} has no thread {tid}"
            )));
        };
        if !comm.starts_with(READER_PREFIX) {
            return Err(Refusal::Refused(
                "only the app's pad reader thread is boosted".into(),
            ));
        }
        if self.proc.uid(pid) == Some(0) {
            return Err(Refusal::Refused("a root process is never boosted".into()));
        }
        if !self.proc.in_app_tree(pid) {
            return Err(Refusal::Refused(format!(
                "the process is not part of {APP_ID}"
            )));
        }
        let Some(started) = self.proc.started(pid, tid) else {
            return Err(Refusal::NotFound(format!(
                "process {pid} has no thread {tid}"
            )));
        };
        let cpus = self.target_cpus();
        let mut entries = self.entries.lock();
        let known = entries.iter().position(|e| e.tid == tid);
        if known.is_none() && entries.len() >= MAX_BOOSTED {
            return Err(Refusal::Refused(format!(
                "{MAX_BOOSTED} threads are boosted already"
            )));
        }
        let saved = match known {
            Some(i) if entries[i].pid == pid && entries[i].started == started => {
                entries[i].saved.clone()
            }
            _ => self
                .sched
                .get(tid)
                .map_err(|e| Refusal::Sched(format!("reading the thread's scheduling: {e}")))?,
        };
        self.sched
            .boost(tid, PRIORITY, cpus.as_deref())
            .map_err(|e| Refusal::Sched(format!("boosting the thread: {e}")))?;
        let entry = Entry {
            pid,
            tid,
            started,
            saved,
            lease_ends: now + LEASE,
        };
        match known {
            Some(i) => entries[i] = entry,
            None => entries.push(entry),
        }
        Ok(
            json!({"tid": tid, "priority": PRIORITY, "cpus": cpus.unwrap_or_default(), "leaseSecs": LEASE.as_secs()}),
        )
    }

    /// End a boost this module made; `false` when there was none (nothing is touched then).
    pub fn unboost(&self, pid: i32, tid: i32) -> bool {
        let mut entries = self.entries.lock();
        let Some(i) = entries.iter().position(|e| e.tid == tid && e.pid == pid) else {
            return false;
        };
        let e = entries.remove(i);
        self.release(&e);
        true
    }

    fn alive(&self, e: &Entry) -> bool {
        self.proc.started(e.pid, e.tid).as_deref() == Some(e.started.as_str())
            && self
                .proc
                .comm(e.pid, e.tid)
                .is_some_and(|c| c.starts_with(READER_PREFIX))
    }

    /// Put a surviving thread back as it was; a gone one needs nothing.
    fn release(&self, e: &Entry) {
        if self.alive(e) {
            let _ = self.sched.restore(e.tid, &e.saved);
        }
    }

    /// Drop what ended: a thread that is gone or renamed, a lease that ran out.
    pub fn sweep(&self, now: Instant) {
        let mut entries = self.entries.lock();
        let mut keep = Vec::with_capacity(entries.len());
        for e in entries.drain(..) {
            if !self.alive(&e) {
                continue;
            }
            if now >= e.lease_ends {
                self.release(&e);
                continue;
            }
            keep.push(e);
        }
        *entries = keep;
    }

    pub fn status(&self) -> Value {
        let entries = self.entries.lock();
        json!(
            entries
                .iter()
                .map(|e| json!({"pid": e.pid, "tid": e.tid, "priority": PRIORITY}))
                .collect::<Vec<_>>()
        )
    }

    pub fn count(&self) -> usize {
        self.entries.lock().len()
    }
}

/// The helper's one boost table; the sweeper starts with the first request.
static GLOBAL: LazyLock<Arc<Boost>> = LazyLock::new(|| {
    let boost = Arc::new(Boost::new(
        "/proc",
        "/sys/devices/system/cpu",
        Box::new(RealSched),
    ));
    let sweeper = Arc::clone(&boost);
    // One second is how fast a lease or a dead thread is noticed; an idle sweep reads nothing when no thread is boosted.
    let _ = std::thread::Builder::new()
        .name("gfn-boost-sweep".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                sweeper.sweep(Instant::now());
            }
        });
    boost
});

fn ids(request: &Request) -> Result<(i32, i32), Response> {
    let v: Value = serde_json::from_slice(&request.body)
        .map_err(|_| error_response(400, "bad_request", "the body is {\"pid\": n, \"tid\": n}"))?;
    let get = |k: &str| v[k].as_i64().and_then(|n| i32::try_from(n).ok());
    match (get("pid"), get("tid")) {
        (Some(pid), Some(tid)) => Ok((pid, tid)),
        _ => Err(error_response(
            400,
            "bad_request",
            "the body is {\"pid\": n, \"tid\": n}",
        )),
    }
}

fn ok(mut body: Value) -> Response {
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".into(), Value::Bool(true));
        map.insert("api".into(), json!(API_VERSION));
        map.insert("now".into(), json!(now_ms()));
    }
    Response::json(200, &body)
}

/// Serves `/v1/gfn/...` on `boost`.
pub fn handle_with(boost: &Boost, request: &Request, rest: &[&str]) -> Response {
    let get = request.method == "GET";
    let post = request.method == "POST";
    match rest {
        ["pad-boost"] if get => ok(json!({"boosted": boost.status()})),
        ["pad-boost"] if post => match ids(request) {
            Ok((pid, tid)) => match boost.boost(pid, tid, Instant::now()) {
                Ok(b) => ok(json!({"boosted": b})),
                Err(why) => why.response(),
            },
            Err(r) => r,
        },
        ["pad-unboost"] if post => match ids(request) {
            Ok((pid, tid)) => ok(json!({"released": boost.unboost(pid, tid)})),
            Err(r) => r,
        },
        ["pad-boost" | "pad-unboost"] => {
            error_response(405, "method_not_allowed", "that method is not served here").with_header(
                "Allow",
                if rest[0] == "pad-boost" {
                    "GET, POST"
                } else {
                    "POST"
                },
            )
        }
        _ => error_response(404, "not_found", "no such endpoint"),
    }
}

pub fn handle(request: &Request, rest: &[&str]) -> Response {
    handle_with(&GLOBAL, request, rest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::sync::Arc as StdArc;

    /// A scheduler that remembers: each thread's current state, and every call.
    #[derive(Default)]
    struct Fake {
        state: Mutex<HashMap<i32, Saved>>,
        calls: Mutex<Vec<String>>,
        fail_boost: Mutex<bool>,
    }

    struct FakeRef(StdArc<Fake>);

    impl Sched for FakeRef {
        fn get(&self, tid: i32) -> io::Result<Saved> {
            self.0.calls.lock().push(format!("get {tid}"));
            Ok(self.0.state.lock().get(&tid).cloned().unwrap_or(Saved {
                policy: 0,
                priority: 0,
                cpus: vec![0, 1, 2, 3],
            }))
        }
        fn boost(&self, tid: i32, priority: i32, cpus: Option<&[usize]>) -> io::Result<()> {
            self.0
                .calls
                .lock()
                .push(format!("boost {tid} {priority} {cpus:?}"));
            if *self.0.fail_boost.lock() {
                return Err(io::Error::from_raw_os_error(libc::EPERM));
            }
            let mut st = self.0.state.lock();
            let cur = st.get(&tid).cloned().unwrap_or(Saved {
                policy: 0,
                priority: 0,
                cpus: vec![0, 1, 2, 3],
            });
            st.insert(
                tid,
                Saved {
                    policy: libc::SCHED_FIFO | libc::SCHED_RESET_ON_FORK,
                    priority,
                    cpus: cpus.map(<[usize]>::to_vec).unwrap_or(cur.cpus),
                },
            );
            Ok(())
        }
        fn restore(&self, tid: i32, saved: &Saved) -> io::Result<()> {
            self.0.calls.lock().push(format!("restore {tid}"));
            self.0.state.lock().insert(tid, saved.clone());
            Ok(())
        }
    }

    struct Rig {
        dir: tempfile::TempDir,
        fake: StdArc<Fake>,
        boost: Boost,
    }

    impl Rig {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("proc")).unwrap();
            fs::create_dir_all(dir.path().join("cpu")).unwrap();
            fs::write(dir.path().join("cpu/online"), "0-3\n").unwrap();
            let fake = StdArc::new(Fake::default());
            let boost = Boost::new(
                dir.path().join("proc"),
                dir.path().join("cpu"),
                Box::new(FakeRef(StdArc::clone(&fake))),
            );
            Self { dir, fake, boost }
        }

        /// A process: its parent, uid, and executable path and arguments.
        fn process(&self, pid: i32, ppid: i32, uid: u32, exe: &str, args: &[&str]) {
            let d = self.dir.path().join("proc").join(pid.to_string());
            fs::create_dir_all(d.join("task")).unwrap();
            fs::write(
                d.join("stat"),
                format!(
                    "{pid} (opennow-stream) S {ppid} 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 5000 0 0"
                ),
            )
            .unwrap();
            fs::write(
                d.join("status"),
                format!("Name:\tx\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
            )
            .unwrap();
            fs::write(d.join("cmdline"), args.join("\0")).unwrap();
            let _ = fs::remove_file(d.join("exe"));
            symlink(exe, d.join("exe")).unwrap();
        }

        fn thread(&self, pid: i32, tid: i32, name: &str, started: u64) {
            let d = self
                .dir
                .path()
                .join("proc")
                .join(pid.to_string())
                .join("task")
                .join(tid.to_string());
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("comm"), format!("{name}\n")).unwrap();
            fs::write(
                d.join("stat"),
                format!("{tid} ({name}) S 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 {started} 0 0"),
            )
            .unwrap();
        }

        fn remove_thread(&self, pid: i32, tid: i32) {
            fs::remove_dir_all(
                self.dir
                    .path()
                    .join("proc")
                    .join(pid.to_string())
                    .join("task")
                    .join(tid.to_string()),
            )
            .unwrap();
        }

        fn app_engine(&self) {
            // sam's main (the feeder) -> the core -> the engine through the package's loader: the id is in the loader's arguments.
            self.process(100, 1, 5227, "/media/developer/apps/usr/palm/applications/org.nphil.lumatv.gfn/lumatv-gfn-feeder", &["lumatv-gfn-feeder"]);
            self.process(101, 100, 5227, "/media/developer/apps/usr/palm/applications/org.nphil.lumatv.gfn/bin64/opennow-core", &["opennow-core"]);
            self.process(
                102,
                101,
                5227,
                "/usr/lib/ld-linux-aarch64.so.1",
                &["ld-linux", "--library-path", "x", "/media/developer/apps/usr/palm/applications/org.nphil.lumatv.gfn/bin64/opennow-streamer.bin"],
            );
        }
    }

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn the_apps_reader_thread_is_boosted_to_fifo_10_off_cpu_0_and_put_back_on_unboost() {
        let rig = Rig::new();
        rig.app_engine();
        rig.thread(102, 2001, "gfn-pad-event15", 7);
        let out = rig.boost.boost(102, 2001, now()).unwrap();
        assert_eq!(out["priority"], 10);
        assert_eq!(out["cpus"], json!([1, 2, 3]));
        let st = rig.fake.state.lock().get(&2001).cloned().unwrap();
        assert_eq!(st.policy, libc::SCHED_FIFO | libc::SCHED_RESET_ON_FORK);
        assert_eq!((st.priority, st.cpus), (10, vec![1, 2, 3]));
        // a second request is a renewal: the original state is kept, not the boosted one
        rig.boost.boost(102, 2001, now()).unwrap();
        assert_eq!(rig.boost.count(), 1);
        assert!(rig.boost.unboost(102, 2001));
        assert_eq!(
            rig.fake.state.lock().get(&2001).unwrap(),
            &Saved {
                policy: 0,
                priority: 0,
                cpus: vec![0, 1, 2, 3]
            }
        );
        assert!(!rig.boost.unboost(102, 2001), "nothing left to release");
    }

    #[test]
    fn only_online_cpus_of_1_to_3_are_used_and_cpu_1_is_required() {
        let rig = Rig::new();
        rig.app_engine();
        rig.thread(102, 2001, "gfn-pad-event15", 7);
        fs::write(rig.dir.path().join("cpu/online"), "0-1\n").unwrap();
        assert_eq!(
            rig.boost.boost(102, 2001, now()).unwrap()["cpus"],
            json!([1])
        );
        fs::write(rig.dir.path().join("cpu/online"), "0,1,3\n").unwrap();
        assert_eq!(
            rig.boost.boost(102, 2001, now()).unwrap()["cpus"],
            json!([1, 3]),
            "a renewal follows the hot-plug"
        );
        fs::write(rig.dir.path().join("cpu/online"), "0\n").unwrap();
        assert_eq!(
            rig.boost.boost(102, 2001, now()).unwrap()["cpus"],
            json!([])
        );
        assert!(
            rig.fake.calls.lock().last().unwrap().ends_with("None"),
            "no affinity change on a one-core set"
        );
        assert_eq!(parse_cpu_list("0-1,3"), vec![0, 1, 3]);
    }

    #[test]
    fn a_foreign_process_is_refused_even_with_the_readers_name() {
        let rig = Rig::new();
        rig.process(500, 1, 1000, "/usr/bin/some-daemon", &["some-daemon"]);
        rig.thread(500, 5001, "gfn-pad-event15", 7);
        assert!(
            matches!(rig.boost.boost(500, 5001, now()), Err(Refusal::Refused(m)) if m.contains(APP_ID))
        );
        assert!(rig.fake.state.lock().is_empty() && rig.boost.count() == 0);
        // a look-alike directory name is not the app
        rig.process(501, 1, 1000, "/media/org.nphil.lumatv.gfn.evil/x", &["x"]);
        rig.thread(501, 5002, "gfn-pad-event15", 7);
        assert!(matches!(
            rig.boost.boost(501, 5002, now()),
            Err(Refusal::Refused(_))
        ));
        // a thread of nobody's
        assert!(matches!(
            rig.boost.boost(500, 9999, now()),
            Err(Refusal::NotFound(_))
        ));
        // init and nonsense
        assert!(matches!(
            rig.boost.boost(1, 1, now()),
            Err(Refusal::Refused(_))
        ));
        assert!(matches!(
            rig.boost.boost(-4, 5001, now()),
            Err(Refusal::Refused(_))
        ));
    }

    #[test]
    fn a_thread_of_the_app_with_another_name_is_refused() {
        let rig = Rig::new();
        rig.app_engine();
        for (tid, name) in [
            (3001, "tokio-runtime-w"),
            (3002, "opennow-protocol"),
            (3003, "gfn-pad-discovery"),
            (3004, "gfn-pad-"),
        ] {
            rig.thread(102, tid, name, 7);
            assert!(
                matches!(rig.boost.boost(102, tid, now()), Err(Refusal::Refused(m)) if m.contains("pad reader")),
                "{name}"
            );
        }
        assert!(rig.fake.state.lock().is_empty());
    }

    #[test]
    fn a_thread_under_a_child_of_the_apps_tree_counts_and_a_root_one_does_not() {
        let rig = Rig::new();
        rig.app_engine();
        // a helper process the engine started: its parent chain reaches the app
        rig.process(103, 102, 5227, "/bin/sh", &["sh"]);
        rig.thread(103, 4001, "gfn-pad-event12", 7);
        assert!(rig.boost.boost(103, 4001, now()).is_ok());
        rig.process(104, 102, 0, "/bin/sh", &["sh"]);
        rig.thread(104, 4002, "gfn-pad-event12", 7);
        assert!(
            matches!(rig.boost.boost(104, 4002, now()), Err(Refusal::Refused(m)) if m.contains("root"))
        );
        // an ancestor beyond six levels does not count
        let mut parent = 103;
        for pid in 200..207 {
            rig.process(pid, parent, 5227, "/bin/sh", &["sh"]);
            parent = pid;
        }
        rig.process(300, 206, 5227, "/bin/sh", &["sh"]);
        rig.thread(300, 6001, "gfn-pad-event12", 7);
        assert!(matches!(
            rig.boost.boost(300, 6001, now()),
            Err(Refusal::Refused(_))
        ));
    }

    #[test]
    fn at_most_four_threads_are_boosted() {
        let rig = Rig::new();
        rig.app_engine();
        for i in 0..5 {
            rig.thread(102, 2000 + i, &format!("gfn-pad-event{i}"), 7);
        }
        for i in 0..4 {
            rig.boost.boost(102, 2000 + i, now()).unwrap();
        }
        assert!(
            matches!(rig.boost.boost(102, 2004, now()), Err(Refusal::Refused(m)) if m.contains("already"))
        );
        assert!(
            rig.boost.boost(102, 2000, now()).is_ok(),
            "a renewal is not a fifth"
        );
    }

    #[test]
    fn a_lease_that_runs_out_a_gone_thread_and_a_reused_number_all_clean_up() {
        let rig = Rig::new();
        rig.app_engine();
        rig.thread(102, 2001, "gfn-pad-event15", 7);
        rig.thread(102, 2002, "gfn-pad-event12", 7);
        rig.thread(102, 2003, "gfn-pad-event13", 7);
        let t0 = now();
        for tid in [2001, 2002, 2003] {
            rig.boost.boost(102, tid, t0).unwrap();
        }
        // 2002 ends, 2003's number is reused by another thread (a different start time and name)
        rig.remove_thread(102, 2002);
        rig.remove_thread(102, 2003);
        rig.thread(102, 2003, "worker", 99);
        rig.boost.sweep(t0 + Duration::from_secs(1));
        assert_eq!(rig.boost.count(), 1);
        let calls = rig.fake.calls.lock().clone();
        assert!(
            !calls
                .iter()
                .any(|c| c == "restore 2002" || c == "restore 2003"),
            "a thread that is gone, or someone else's now, is not touched: {calls:?}"
        );
        // 2001 lives on until its lease ends, then is restored
        rig.boost.sweep(t0 + LEASE - Duration::from_secs(1));
        assert_eq!(rig.boost.count(), 1);
        rig.boost.sweep(t0 + LEASE);
        assert_eq!(rig.boost.count(), 0);
        assert_eq!(rig.fake.state.lock().get(&2001).unwrap().policy, 0);
        // a renewal moves the end of the lease
        rig.boost.boost(102, 2001, t0 + LEASE).unwrap();
        rig.boost.sweep(t0 + LEASE + Duration::from_secs(60));
        assert_eq!(rig.boost.count(), 1);
    }

    #[test]
    fn a_failed_scheduler_call_is_reported_and_leaves_nothing_boosted() {
        let rig = Rig::new();
        rig.app_engine();
        rig.thread(102, 2001, "gfn-pad-event15", 7);
        *rig.fake.fail_boost.lock() = true;
        assert!(matches!(
            rig.boost.boost(102, 2001, now()),
            Err(Refusal::Sched(_))
        ));
        assert_eq!(rig.boost.count(), 0);
    }

    fn request(method: &str, path: &str, body: &str) -> Request {
        Request {
            method: method.into(),
            path: path.into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: body.as_bytes().to_vec(),
            peer: "127.0.0.1".parse().unwrap(),
        }
    }

    fn body(r: &Response) -> Value {
        serde_json::from_slice(&r.body).unwrap()
    }

    #[test]
    fn the_routes_answer_with_the_apis_envelope_and_codes() {
        let rig = Rig::new();
        rig.app_engine();
        rig.thread(102, 2001, "gfn-pad-event15", 7);
        rig.process(500, 1, 1000, "/usr/bin/x", &["x"]);
        rig.thread(500, 5001, "gfn-pad-event15", 7);
        let h = |m: &str, p: &str, b: &str| {
            let segments: Vec<&str> = p.trim_matches('/').split('/').skip(2).collect();
            handle_with(&rig.boost, &request(m, p, b), &segments)
        };
        let r = h("POST", "/v1/gfn/pad-boost", r#"{"pid":102,"tid":2001}"#);
        assert_eq!(r.status, 200);
        assert_eq!(body(&r)["boosted"]["priority"], 10);
        assert_eq!(
            body(&h("GET", "/v1/gfn/pad-boost", ""))["boosted"][0]["tid"],
            2001
        );
        let r = h("POST", "/v1/gfn/pad-boost", r#"{"pid":500,"tid":5001}"#);
        assert_eq!(
            (r.status, body(&r)["error"]["code"].clone()),
            (403, json!("refused"))
        );
        assert_eq!(
            h("POST", "/v1/gfn/pad-boost", r#"{"pid":102,"tid":77}"#).status,
            404
        );
        assert_eq!(h("POST", "/v1/gfn/pad-boost", "{}").status, 400);
        assert_eq!(
            h("POST", "/v1/gfn/pad-boost", r#"{"pid":"102","tid":2001}"#).status,
            400
        );
        assert_eq!(h("DELETE", "/v1/gfn/pad-boost", "").status, 405);
        assert_eq!(h("GET", "/v1/gfn/pad-unboost", "").status, 405);
        assert_eq!(h("GET", "/v1/gfn/nope", "").status, 404);
        assert_eq!(
            body(&h(
                "POST",
                "/v1/gfn/pad-unboost",
                r#"{"pid":102,"tid":2001}"#
            ))["released"],
            true
        );
        assert_eq!(
            body(&h(
                "POST",
                "/v1/gfn/pad-unboost",
                r#"{"pid":102,"tid":2001}"#
            ))["released"],
            false
        );
    }

    #[test]
    fn the_real_scheduler_reads_this_threads_own_state_and_puts_it_back_unchanged() {
        // Raising to FIFO needs privilege a test does not have; reading and restoring the thread's own state does not.
        let tid = unsafe { libc::syscall(libc::SYS_gettid) } as i32;
        let saved = RealSched.get(tid).unwrap();
        assert_eq!(saved.policy & !libc::SCHED_RESET_ON_FORK, libc::SCHED_OTHER);
        assert!(!saved.cpus.is_empty());
        RealSched.restore(tid, &saved).unwrap();
        assert_eq!(RealSched.get(tid).unwrap(), saved);
    }
}
