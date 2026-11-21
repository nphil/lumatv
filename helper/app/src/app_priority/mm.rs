//! The link to `lumatv-mm` (`helper/mm/lumatv-mm.c`), the shim that serves `com.webos.memorymanager` in LG's memchute's place while
//! LumaTV's memory manager is switched on (`lumatv mm on`: the `enabled` marker in `<data dir>/mm`, read by the shim and by
//! `tools/mm.sh`, so the switch works with the helper stopped).
//!
//! The link connects to the shim's socket, answers its heartbeat on its own thread (so a busy policy never misses one), hands the
//! shim's launches to the engine and the engine's messages to the shim ([`manager`](super::manager)). It also supervises:
//! - a shim that stops pinging for 15 s is killed, and systemd restarts it (`Restart=on-failure`);
//! - each crash record the shim leaves (`crash/pending`) is told once through Home Assistant's notify service
//!   (`appPriority.notify`), naming the reason and when LumaTV's tries again;
//! - while the switch is on but LG's memchute serves (after a fallback), `mm.sh on --auto` swaps back once Home is in front, at
//!   most every minute doubling to ten, and never after the shim gave up for this boot (`gave-up`).

use super::Msg;
use super::manager::Shim;
use crate::ctx::Ctx;
use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, ErrorKind, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const SOCKET: &str = "bridge.sock";
/// The shim pings every 5 s; three missed is a hung shim.
const PING_LOST: Duration = Duration::from_secs(15);
/// After the link went, how long before LG's serving counts as a fallback to undo: a shim restarting connects well inside it.
const SETTLE: Duration = Duration::from_secs(30);
const FIRST_RETRY: Duration = Duration::from_secs(60);
const LAST_RETRY: Duration = Duration::from_secs(600);
const HOME: &str = "com.webos.app.home";
const TITLE: &str = "TV: memory manager handed back to LG's";

/// The shim's side of the bridge as the engine writes to it.
struct Writer(Mutex<Option<UnixStream>>);

impl Shim for Writer {
    fn send(&self, message: Value) {
        let mut guard = self.0.lock();
        let Some(stream) = guard.as_mut() else { return };
        let mut line = message.to_string();
        line.push('\n');
        if let Err(e) = stream.write_all(line.as_bytes()) {
            crate::log_warn!("memory manager: writing to the shim: {e}");
            let _ = stream.shutdown(std::net::Shutdown::Both);
            *guard = None;
        }
    }
}

/// Where the link finds things: the TV's, or a test's.
struct Files {
    /// `enabled`, `gave-up`, `bridge.sock`, `crash/`.
    state: PathBuf,
    mm_sh: PathBuf,
    boot_id: PathBuf,
}

impl Files {
    fn tv(ctx: &Ctx) -> Self {
        let app = ctx.config.brand_app_id.as_deref().unwrap_or("org.nphil.lumatv");
        Self {
            state: ctx.paths.data_dir.join("mm"),
            mm_sh: PathBuf::from(format!("/media/developer/apps/usr/palm/applications/{app}/tools/mm.sh")),
            boot_id: PathBuf::from("/proc/sys/kernel/random/boot_id"),
        }
    }

    fn enabled(&self) -> bool {
        self.state.join("enabled").exists()
    }

    fn gave_up(&self) -> bool {
        let first = |p: &Path| std::fs::read_to_string(p).ok().map(|t| t.lines().next().unwrap_or("").trim().to_owned());
        first(&self.state.join("gave-up")).is_some_and(|g| !g.is_empty() && Some(g) == first(&self.boot_id))
    }
}

/// What the supervisor decided this second.
#[derive(Debug, PartialEq)]
enum Act {
    Rearm,
    Tell(String),
}

/// The supervision, on a clock given to it.
struct Keeper {
    files: Files,
    /// The crash record last told, kept in `crash/told` so a helper restart does not tell it again.
    told: Option<String>,
    down_since: Option<Instant>,
    tries: u32,
    next_try: Option<Instant>,
}

impl Keeper {
    fn new(files: Files) -> Self {
        let told = std::fs::read_to_string(files.state.join("crash/told")).ok().map(|t| t.trim().to_owned());
        Self { files, told, down_since: None, tries: 0, next_try: None }
    }

    fn step(&mut self, now: Instant, connected: bool, home: bool) -> Vec<Act> {
        let mut acts = Vec::new();
        if let Some(record) = self.news() {
            acts.push(Act::Tell(record));
        }
        if connected {
            (self.down_since, self.tries, self.next_try) = (None, 0, None);
            return acts;
        }
        let since = *self.down_since.get_or_insert(now);
        let due = self.files.enabled()
            && !self.files.gave_up()
            && home
            && now.saturating_duration_since(since) >= SETTLE
            && self.next_try.is_none_or(|t| now >= t);
        if due {
            let wait = FIRST_RETRY.saturating_mul(1 << self.tries.min(4)).min(LAST_RETRY);
            self.tries += 1;
            self.next_try = Some(now + wait);
            acts.push(Act::Rearm);
        }
        acts
    }

    /// A crash record not told yet.
    fn news(&mut self) -> Option<String> {
        let name = std::fs::read_to_string(self.files.state.join("crash/pending")).ok()?.trim().to_owned();
        if name.is_empty() || self.told.as_deref() == Some(name.as_str()) {
            return None;
        }
        let _ = std::fs::write(self.files.state.join("crash/told"), &name);
        self.told = Some(name.clone());
        Some(name)
    }

    fn message(&self) -> Option<String> {
        let notice = crate::ssgw::pending(&self.files.state.join("crash"))?;
        let next = if self.files.gave_up() {
            "LG's memory manager serves until the TV restarts or `lumatv mm on`."
        } else {
            "LG's memory manager serves; LumaTV's tries again when Home is next in front."
        };
        Some(format!("{}. {}. {next} {}.", notice.line, notice.error, notice.place))
    }

    fn status(&self) -> Value {
        let notice = crate::ssgw::pending(&self.files.state.join("crash"));
        json!({
            "switch": self.files.enabled(),
            "gaveUp": self.files.gave_up(),
            "crash": notice.map(|n| format!("{} ({})", n.line, n.error)),
        })
    }
}

struct Shared {
    stop: AtomicBool,
    home: AtomicBool,
    status: Mutex<Value>,
    rearming: AtomicBool,
}

pub struct Link {
    shared: Arc<Shared>,
    writer: Arc<Writer>,
    thread: Option<JoinHandle<()>>,
}

impl Link {
    pub(super) fn spawn(ctx: &Ctx, tx: mpsc::Sender<Msg>, policy: ha_link::AddressPolicy) -> Self {
        let shared = Arc::new(Shared {
            stop: AtomicBool::new(false),
            home: AtomicBool::new(false),
            status: Mutex::new(Value::Null),
            rearming: AtomicBool::new(false),
        });
        let writer = Arc::new(Writer(Mutex::new(None)));
        let files = Files::tv(ctx);
        let tell = {
            let links = vec![ctx.paths.ha_link_file(), ctx.paths.data_dir.join("voice").join("ha.json")];
            let (config, service) = (ctx.config.clone(), ctx.config.app_priority.notify.clone());
            move |message: String| {
                if service.is_empty() {
                    return;
                }
                let (links, config, service) = (links.clone(), config.clone(), service.clone());
                let spawned = thread::Builder::new().name("mm-notify".into()).stack_size(512 * 1024).spawn(move || {
                    if let Err(e) = crate::ssgw::notify(&links, &config, policy, &service, TITLE, &message) {
                        crate::log_warn!("memory manager: the phone was not told: {e}");
                    }
                });
                if let Err(e) = spawned {
                    crate::log_warn!("memory manager: the phone was not told: {e}");
                }
            }
        };
        let (s, w) = (shared.clone(), writer.clone());
        let thread = thread::Builder::new()
            .name("mm-link".into())
            .stack_size(128 * 1024)
            .spawn(move || serve(&s, &w, files, &tx, &tell))
            .map_err(|e| crate::log_warn!("memory manager: the link's thread: {e}"))
            .ok();
        Self { shared, writer, thread }
    }

    /// Adds the link's part to the policy's status, and reads from it whether Home is in front.
    pub(super) fn annotate(&self, status: &mut Value) {
        let front = status.get("front").and_then(Value::as_array);
        let home = front.is_none_or(|f| f.first().and_then(Value::as_str).is_none_or(|a| a == HOME || a.starts_with(super::OURS_PREFIX)));
        self.shared.home.store(home, Ordering::Relaxed);
        if let (Some(mm), Value::Object(link)) = (status.get_mut("memoryManager").and_then(Value::as_object_mut), self.shared.status.lock().clone()) {
            mm.extend(link);
        }
    }

    /// The helper is stopping: the shim waits for it longer and then gives the name back without counting a failure.
    pub(super) fn stop(mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.writer.send(json!({"kind": "bye"}));
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn serve(shared: &Arc<Shared>, writer: &Arc<Writer>, files: Files, tx: &mpsc::Sender<Msg>, tell: &dyn Fn(String)) {
    let socket = files.state.join(SOCKET);
    let mm_sh = files.mm_sh.clone();
    let mut keeper = Keeper::new(files);
    let mut reader: Option<BufReader<UnixStream>> = None;
    let mut line = Vec::new();
    let mut peer: Option<i32> = None;
    let mut last_ping = Instant::now();
    while !shared.stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if reader.is_none() {
            if let Some(stream) = connect(&socket) {
                *writer.0.lock() = stream.try_clone().ok();
                reader = Some(BufReader::new(stream));
                (peer, last_ping) = (None, now);
                line.clear();
            }
        }
        let mut lost = None;
        if let Some(r) = &mut reader {
            match r.read_until(b'\n', &mut line) {
                Ok(0) => lost = Some("the shim closed the link"),
                Ok(_) if line.ends_with(b"\n") => {
                    match serde_json::from_slice::<Value>(&line) {
                        Ok(m) => handle(&m, writer, tx, &mut peer, &mut last_ping),
                        Err(_) => crate::log_warn!("memory manager: an unreadable line from the shim"),
                    }
                    line.clear();
                    continue;
                }
                Ok(_) => {}
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
                Err(_) => lost = Some("the link to the shim broke"),
            }
            if lost.is_none() && writer.0.lock().is_none() {
                lost = Some("the link to the shim broke");
            }
            if lost.is_none() && now.saturating_duration_since(last_ping) >= PING_LOST {
                if let Some(pid) = peer.filter(|&p| is_shim(p)) {
                    crate::log_warn!("memory manager: the shim ({pid}) stopped pinging; killed for systemd to restart it");
                    // SAFETY: `kill` has no memory preconditions; the pid was just checked to be the shim.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
                lost = Some("the shim stopped answering");
            }
        } else {
            thread::sleep(Duration::from_secs(1));
        }
        if let Some(why) = lost {
            reader = None;
            *writer.0.lock() = None;
            line.clear();
            if peer.is_some() {
                let _ = tx.send(Msg::MmDown(why.to_owned()));
            }
            peer = None;
        }
        for act in keeper.step(Instant::now(), peer.is_some(), shared.home.load(Ordering::Relaxed)) {
            match act {
                Act::Tell(record) => {
                    crate::log_warn!("memory manager: the shim handed back to LG's ({record})");
                    if let Some(message) = keeper.message() {
                        tell(message);
                    }
                }
                Act::Rearm if !shared.rearming.swap(true, Ordering::Relaxed) => rearm(&mm_sh, shared),
                Act::Rearm => {}
            }
        }
        *shared.status.lock() = keeper.status();
    }
    if peer.is_some() {
        let _ = tx.send(Msg::MmDown("the helper stopped".to_owned()));
    }
}

fn connect(socket: &Path) -> Option<UnixStream> {
    let stream = UnixStream::connect(socket).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(1))).ok()?;
    Some(stream)
}

fn handle(m: &Value, writer: &Arc<Writer>, tx: &mpsc::Sender<Msg>, peer: &mut Option<i32>, last_ping: &mut Instant) {
    match m.get("kind").and_then(Value::as_str) {
        Some("hello") => {
            *peer = m.get("pid").and_then(Value::as_i64).and_then(|p| i32::try_from(p).ok());
            *last_ping = Instant::now();
            crate::log_info!("memory manager: linked to the shim ({})", peer.unwrap_or(0));
            let shim: Arc<dyn Shim> = writer.clone();
            let _ = tx.send(Msg::MmUp(shim));
        }
        Some("ping") => {
            *last_ping = Instant::now();
            writer.send(json!({"kind": "pong", "seq": m.get("seq").cloned().unwrap_or(Value::Null)}));
        }
        Some("launch") => {
            if let Some(payload) = m.get("payload") {
                let _ = tx.send(Msg::Launch(payload.clone()));
            }
        }
        other => crate::log_warn!("memory manager: the shim said {other:?}"),
    }
}

/// The process is the shim (or LG's memchute, which the shim execs into; neither pings then).
fn is_shim(pid: i32) -> bool {
    pid > 1 && std::fs::read_to_string(format!("/proc/{pid}/comm")).is_ok_and(|c| c.trim() == "memchute")
}

/// `mm.sh on --auto` on a thread of its own: it restarts LG's unit and waits for it.
fn rearm(mm_sh: &Path, shared: &Arc<Shared>) {
    let (mm_sh, done) = (mm_sh.to_owned(), shared.clone());
    let spawned = thread::Builder::new().name("mm-rearm".into()).stack_size(64 * 1024).spawn(move || {
        crate::log_info!("memory manager: switching back to LumaTV's");
        match Command::new("/bin/sh").arg(&mm_sh).args(["on", "--auto"]).stdin(Stdio::null()).stderr(Stdio::null()).output() {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout);
                crate::log_info!("memory manager: mm.sh on: {}", text.lines().last().unwrap_or(""));
            }
            Err(e) => crate::log_warn!("memory manager: mm.sh on: {e}"),
        }
        done.rearming.store(false, Ordering::Relaxed);
    });
    if let Err(e) = spawned {
        crate::log_warn!("memory manager: mm.sh on: {e}");
        shared.rearming.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(dir: &Path) -> Files {
        std::fs::create_dir_all(dir.join("mm/crash")).unwrap();
        std::fs::write(dir.join("boot_id"), "boot-a\n").unwrap();
        Files { state: dir.join("mm"), mm_sh: dir.join("mm.sh"), boot_id: dir.join("boot_id") }
    }

    #[test]
    fn lg_serving_with_the_switch_on_is_undone_with_home_in_front_backing_off() {
        let dir = tempfile::tempdir().unwrap();
        let mut k = Keeper::new(files(dir.path()));
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // Switched off: nothing.
        assert!(k.step(at(0), false, true).is_empty());
        assert!(k.step(at(40), false, true).is_empty());
        std::fs::write(dir.path().join("mm/enabled"), "").unwrap();
        // Not before Home is in front, then once, then a minute on, then two.
        assert!(k.step(at(41), false, false).is_empty());
        assert_eq!(k.step(at(42), false, true), vec![Act::Rearm]);
        assert!(k.step(at(43), false, true).is_empty());
        assert_eq!(k.step(at(102), false, true), vec![Act::Rearm]);
        assert!(k.step(at(161), false, true).is_empty());
        assert_eq!(k.step(at(222), false, true), vec![Act::Rearm]);
        // Linked: the backoff starts over, and a fresh loss waits out the settling time.
        assert!(k.step(at(230), true, true).is_empty());
        assert!(k.step(at(231), false, true).is_empty());
        assert_eq!(k.step(at(261), false, true), vec![Act::Rearm]);
        // Given up this boot: never; a give-up of another boot does not count.
        std::fs::write(dir.path().join("mm/gave-up"), "boot-a\n").unwrap();
        assert!(k.step(at(2000), false, true).is_empty());
        std::fs::write(dir.path().join("mm/gave-up"), "boot-old\n").unwrap();
        assert_eq!(k.step(at(2001), false, true), vec![Act::Rearm]);
    }

    #[test]
    fn each_crash_record_is_told_once_even_across_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let f = files(dir.path());
        let record = dir.path().join("mm/crash/20261120-1");
        std::fs::create_dir_all(&record).unwrap();
        std::fs::write(record.join("reason.txt"), "summary: LumaTV's memory manager stopped and LG's took over\nlast error: the helper was gone for 10 s\n").unwrap();
        std::fs::write(dir.path().join("mm/crash/pending"), "20261120-1\n").unwrap();
        let mut k = Keeper::new(f);
        let now = Instant::now();
        assert_eq!(k.step(now, true, true), vec![Act::Tell("20261120-1".into())]);
        let message = k.message().unwrap();
        assert!(message.contains("the helper was gone for 10 s") && message.contains("tries again"), "{message}");
        assert!(k.step(now, true, true).is_empty());
        assert!(Keeper::new(files(dir.path())).step(now, true, true).is_empty());
        assert_eq!(k.status()["crash"], json!("LumaTV's memory manager stopped and LG's took over (Last error: the helper was gone for 10 s)"));
    }
}
