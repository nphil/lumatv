//! Starting the programs of the plugins that are switched on, as their own users, and starting them again when they stop.
//!
//! The helper is root on the TV and a plugin must not be: for each plugin of the roster (`enabled.json`, approved by the owner) the
//! runner starts `<plugins dir>/<id>/<run.exe>` as the plugin's user id and the plugins' group, in a session of its own, with a
//! cleared environment, `no_new_privs`, no core dumps, a low priority and ceilings on address space, files, processes and file
//! size. It is not the Show's box (this TV's kernel has no user or network namespaces and no seccomp): the ceilings catch a runaway,
//! and **only reviewed plugins belong in `enabled.json`**.
//!
//! **What it starts is what the owner installed.** The program must be a file inside the plugin's directory, owned by root (or by
//! the helper's own user in a test), writable by nobody else, in directories nobody else can write to, and, when the roster entry
//! names the hash of the installed program, exactly that file. A program that is not (a file swapped after the owner approved) is
//! not started, and the plugin's entry in the `plugins` topic says why.
//!
//! **It starts nothing by itself:** only roster members, only while the broker's socket exists, and a plugin that stops is started
//! again after a pause that doubles from 2 s to a minute (a pause that is not reset until the program has stayed up a minute), so a
//! plugin that dies at once does not spin. What it says goes to `<plugins dir>/<id>/log` (rotated at 256 KiB).

use super::roster::{Member, Roster};
use super::session::Shared;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a plugin that is told to stop has before it is ended.
const GRACE: Duration = Duration::from_secs(2);
/// The size at which a plugin's log becomes `log.1`.
const LOG_BYTES: u64 = 256 * 1024;
/// The most a plugin's program may be (it is read whole to be hashed).
const MAX_PROGRAM_BYTES: u64 = 64 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

/// How long a plugin that stopped waits to be started again.
#[derive(Clone, Copy, Debug)]
pub struct Backoff {
    /// The first pause, doubled after each stop that came too soon.
    pub first: Duration,
    pub longest: Duration,
    /// A program that stayed up this long starts from the first pause again.
    pub stable: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            first: Duration::from_secs(2),
            longest: Duration::from_secs(60),
            stable: Duration::from_secs(60),
        }
    }
}

struct Entry {
    member: Member,
    child: Option<Child>,
    started: Option<Instant>,
    failures: u32,
    next_try: Instant,
}

pub struct Runner {
    dir: PathBuf,
    shared: Arc<Shared>,
    entries: BTreeMap<String, Entry>,
    backoff: Backoff,
}

/// How long a program that has stopped `failures` times in a row waits to be started again.
fn pause(backoff: Backoff, failures: u32) -> Duration {
    let doubled = backoff
        .first
        .saturating_mul(1u32 << failures.saturating_sub(1).min(16));
    doubled.min(backoff.longest)
}

/// The same plugin as before: nothing the runner starts it with has changed.
fn same(a: &Member, b: &Member) -> bool {
    a.uid == b.uid
        && a.program == b.program
        && a.manifest.consent_document() == b.manifest.consent_document()
}

fn owned_by_us(meta: &std::fs::Metadata) -> bool {
    // SAFETY: `geteuid` has no preconditions.
    let me = unsafe { libc::geteuid() };
    meta.uid() == 0 || meta.uid() == me
}

/// The program file of a plugin, checked: inside its directory, a plain executable that nobody but root (or the helper's user) can
/// write, in directories the same holds for, and the installed one when the roster says which that is.
fn checked_program(package: &Path, exe: &str, hash: Option<&str>) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(package).map_err(|e| format!("its directory: {e}"))?;
    let real = std::fs::canonicalize(package.join(exe)).map_err(|e| format!("its program: {e}"))?;
    if !real.starts_with(&root) {
        return Err("its program is outside its directory".into());
    }
    let meta = std::fs::metadata(&real).map_err(|e| format!("its program: {e}"))?;
    if !meta.is_file() || meta.permissions().mode() & 0o111 == 0 {
        return Err("its program is not an executable file".into());
    }
    if meta.len() > MAX_PROGRAM_BYTES {
        return Err("its program is too big".into());
    }
    let mut dirs: Vec<&Path> = real.ancestors().skip(1).collect();
    dirs.retain(|d| d.starts_with(&root));
    for path in std::iter::once(real.as_path()).chain(dirs) {
        let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.permissions().mode() & 0o022 != 0 || !owned_by_us(&meta) {
            return Err(format!(
                "{} can be changed by someone who is not the owner",
                path.display()
            ));
        }
    }
    if let Some(want) = hash {
        let mut text = Vec::new();
        std::fs::File::open(&real)
            .and_then(|f| f.take(MAX_PROGRAM_BYTES).read_to_end(&mut text))
            .map_err(|e| format!("its program: {e}"))?;
        let digest: String = Sha256::digest(&text)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if !digest.eq_ignore_ascii_case(want) {
            return Err("its program is not the one that was installed".into());
        }
    }
    Ok(real)
}

fn rotate_and_open(log: &Path) -> std::io::Result<std::fs::File> {
    if std::fs::metadata(log).is_ok_and(|m| m.len() > LOG_BYTES) {
        let _ = std::fs::rename(log, log.with_extension("1"));
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log)
}

fn set_limit(resource: u32, value: u64) {
    let limit = libc::rlimit {
        rlim_cur: value as libc::rlim_t,
        rlim_max: value as libc::rlim_t,
    };
    // SAFETY: a plain struct; a failure leaves the limit as it was. (The resource's type is the C library's: `as _`.)
    unsafe { libc::setrlimit(resource as _, &limit) };
}

impl Runner {
    pub fn new(dir: PathBuf, shared: Arc<Shared>) -> Self {
        Self::with_backoff(dir, shared, Backoff::default())
    }

    pub fn with_backoff(dir: PathBuf, shared: Arc<Shared>, backoff: Backoff) -> Self {
        Self {
            dir,
            shared,
            entries: BTreeMap::new(),
            backoff,
        }
    }

    /// Makes what runs what the roster says: a program for each member, none for a plugin that is no longer on or has changed.
    pub fn sync(&mut self, roster: &Roster) {
        let gone: Vec<String> = self
            .entries
            .iter()
            .filter(|(id, entry)| {
                roster
                    .members
                    .iter()
                    .find(|m| &m.id == *id)
                    .is_none_or(|m| !same(&entry.member, m))
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in gone {
            if let Some(mut entry) = self.entries.remove(&id) {
                crate::log_info!("plugins: {id}: ending its program");
                stop_child(&mut entry, GRACE);
                self.shared.set_run_note(&id, None);
            }
        }
        for member in &roster.members {
            self.entries
                .entry(member.id.clone())
                .or_insert_with(|| Entry {
                    member: member.clone(),
                    child: None,
                    started: None,
                    failures: 0,
                    next_try: Instant::now(),
                });
        }
        self.tick();
    }

    /// Notices programs that stopped and starts what is due.
    pub fn tick(&mut self) {
        let now = Instant::now();
        let ids: Vec<String> = self.entries.keys().cloned().collect();
        for id in ids {
            let Some(entry) = self.entries.get_mut(&id) else {
                continue;
            };
            if let Some(child) = entry.child.as_mut() {
                let pid = child.id();
                match child.try_wait() {
                    Ok(None) => continue,
                    Ok(Some(status)) => {
                        let lived = entry.started.map_or(Duration::ZERO, |s| s.elapsed());
                        if lived >= self.backoff.stable {
                            entry.failures = 0;
                        }
                        entry.failures += 1;
                        let pause = pause(self.backoff, entry.failures);
                        entry.next_try = now + pause;
                        entry.child = None;
                        sweep(Some(pid));
                        crate::log_warn!(
                            "plugins: {id}: its program stopped ({status}) after {lived:?}; starting it again in {pause:?}"
                        );
                        self.shared.set_run_note(
                            &id,
                            Some(format!(
                                "its program stopped ({status}); it is started again in a moment"
                            )),
                        );
                    }
                    Err(e) => {
                        crate::log_warn!(
                            "plugins: {id}: cannot tell whether its program runs: {e}"
                        );
                        continue;
                    }
                }
            }
            let Some(entry) = self.entries.get_mut(&id) else {
                continue;
            };
            if entry.child.is_none() && now >= entry.next_try {
                match self.start(&id) {
                    Ok(child) => {
                        crate::log_info!("plugins: {id}: started its program (pid {})", child.id());
                        if let Some(entry) = self.entries.get_mut(&id) {
                            entry.child = Some(child);
                            entry.started = Some(Instant::now());
                        }
                        self.shared.set_run_note(&id, None);
                    }
                    Err(why) => {
                        crate::log_warn!("plugins: {id}: its program was not started: {why}");
                        let pause = {
                            let failures = self.entries.get(&id).map_or(1, |e| e.failures + 1);
                            pause(self.backoff, failures)
                        };
                        if let Some(entry) = self.entries.get_mut(&id) {
                            entry.failures += 1;
                            entry.next_try = Instant::now() + pause;
                        }
                        self.shared
                            .set_run_note(&id, Some(format!("not started: {why}")));
                    }
                }
            }
        }
    }

    /// How long until the runner has something to do: soon, while a program runs (it may stop), else when the next start is due.
    pub fn wait_hint(&self, longest: Duration) -> Duration {
        let now = Instant::now();
        self.entries
            .values()
            .map(|e| {
                if e.child.is_some() {
                    Duration::from_secs(2)
                } else {
                    e.next_try.saturating_duration_since(now)
                }
            })
            .min()
            .map_or(longest, |d| d.min(longest))
    }

    /// The plugins whose program runs now.
    pub fn running(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|(_, e)| e.child.is_some())
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Ends every program: told politely (they have been sent `shutdown` by the broker already), then ended once `grace` is up.
    pub fn stop_all(&mut self, grace: Duration) {
        let mut children: Vec<(String, Child)> = Vec::new();
        for (id, entry) in std::mem::take(&mut self.entries) {
            if let Some(child) = entry.child {
                crate::log_info!("plugins: {id}: ending its program");
                if let Ok(pid) = libc::pid_t::try_from(child.id()) {
                    // SAFETY: a signal to a process of ours.
                    unsafe { libc::kill(pid, libc::SIGTERM) };
                }
                children.push((id, child));
            }
        }
        // They are all told at once, so the grace is spent once and not once each.
        let end = Instant::now() + grace;
        while Instant::now() < end && !children.is_empty() {
            children.retain_mut(|(_, child)| !matches!(child.try_wait(), Ok(Some(_))));
            std::thread::sleep(Duration::from_millis(20));
        }
        for (_, mut child) in children {
            sweep(Some(child.id()));
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    // The resource limits' constants are the C library's type (unsigned on glibc, `int` on musl, which is the TV's): the casts below
    // say one type for both.
    #[allow(clippy::unnecessary_cast)]
    fn start(&self, id: &str) -> Result<Child, String> {
        let entry = self.entries.get(id).ok_or("not a plugin of the roster")?;
        let member = &entry.member;
        let package = self.dir.join(id);
        let program = checked_program(
            &package,
            &member.manifest.run.exe,
            member.program.as_deref(),
        )?;
        // SAFETY: `geteuid` has no preconditions.
        let me = unsafe { libc::geteuid() };
        let root = me == 0;
        if !root && member.uid != me {
            return Err("only root can run a plugin as a user of its own".into());
        }
        let gid = plugin_api::PLUGINS_GID;
        let data = package.join("data");
        std::fs::create_dir_all(&data).map_err(|e| format!("its data directory: {e}"))?;
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("its data directory: {e}"))?;
        if root {
            std::os::unix::fs::chown(&data, Some(member.uid), Some(gid))
                .map_err(|e| format!("its data directory: {e}"))?;
        }
        let log = rotate_and_open(&package.join("log")).map_err(|e| format!("its log: {e}"))?;
        let log2 = log.try_clone().map_err(|e| format!("its log: {e}"))?;
        let class = member.manifest.class;
        let address_space = match class {
            plugin_api::Class::S => 128 * MIB,
            plugin_api::Class::M => 256 * MIB,
            plugin_api::Class::L => 512 * MIB,
        };
        let uid = member.uid;
        let mut command = Command::new(&program);
        command
            .args(&member.manifest.run.args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &data)
            .env("LUMATV_PLUGIN", id)
            .current_dir(&data)
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(log2);
        // SAFETY: only async-signal-safe calls (libc system calls on plain integers) run between fork and exec.
        unsafe {
            command.pre_exec(move || {
                libc::setsid();
                libc::setpriority(libc::PRIO_PROCESS as _, 0, 10);
                // The address space is a ceiling that catches a runaway, not the class's memory budget: stacks and the allocator
                // reserve far more than a plugin touches.
                set_limit(libc::RLIMIT_AS as u32, address_space);
                set_limit(libc::RLIMIT_NOFILE as u32, 64);
                set_limit(libc::RLIMIT_CORE as u32, 0);
                set_limit(libc::RLIMIT_FSIZE as u32, MIB);
                set_limit(libc::RLIMIT_NPROC as u32, 64);
                if root {
                    let groups = [gid];
                    if libc::setgroups(1, groups.as_ptr()) != 0
                        || libc::setgid(gid) != 0
                        || libc::setuid(uid) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
                libc::prctl(libc::PR_SET_DUMPABLE, 0, 0, 0, 0);
                // If the helper dies without ending its programs (SIGKILL after eight seconds, a crash) they are told to finish,
                // so none is left to meet the next helper's copy of itself at the broker's door.
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM, 0, 0, 0);
                Ok(())
            });
        }
        command.spawn().map_err(|e| format!("{e}"))
    }
}

/// Ends whatever else is left in a program's session (a plugin may start processes of its own).
fn sweep(pid: Option<u32>) {
    if let Some(pid) = pid.and_then(|p| libc::pid_t::try_from(p).ok()) {
        // SAFETY: a signal to a process group; the group is the dead program's own.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
}

/// Asks the program to finish (SIGTERM, which the SDK turns into the plugin's `shutdown`) and ends it, and what it started, when
/// `grace` is up.
fn stop_child(entry: &mut Entry, grace: Duration) {
    let Some(mut child) = entry.child.take() else {
        return;
    };
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: a signal to a process of ours.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
    let end = Instant::now() + grace;
    while Instant::now() < end {
        if matches!(child.try_wait(), Ok(Some(_))) {
            sweep(Some(child.id()));
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    sweep(Some(child.id()));
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hub::Hub;
    use crate::settings::SettingsStore;
    use plugin_api::Manifest;
    use std::os::unix::fs::PermissionsExt;

    fn my_uid() -> u32 {
        // SAFETY: getuid cannot fail.
        unsafe { libc::getuid() }
    }

    fn shared(dir: &Path) -> Arc<Shared> {
        let hub = Arc::new(Hub::new(1));
        Shared::new(
            hub.clone(),
            Arc::new(SettingsStore::new(dir.join("settings"), hub)),
            Duration::from_secs(1),
            Arc::new(|| 0),
        )
    }

    /// A plugin directory whose program is `script`, switched on as `uid`.
    fn plugin(dir: &Path, id: &str, script: &str, uid: u32) -> Member {
        let package = dir.join("plugins").join(id);
        std::fs::create_dir_all(package.join("bin")).unwrap();
        let program = package.join("bin").join("run.sh");
        std::fs::write(&program, script).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        for d in [dir, &dir.join("plugins"), &package, &package.join("bin")] {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let text = format!(
            r#"{{"schema":1,"id":"{id}","name":"T","version":"1","api":{{"min":1,"target":1}},"run":{{"exe":"bin/run.sh"}},"class":"S","permissions":["cards"]}}"#
        );
        Member {
            id: id.into(),
            uid,
            manifest: Manifest::parse(text.as_bytes()).unwrap(),
            program: None,
        }
    }

    fn roster(members: Vec<Member>) -> Roster {
        Roster {
            members,
            ..Roster::default()
        }
    }

    fn wait(mut done: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn data(dir: &Path, id: &str, file: &str) -> PathBuf {
        dir.join("plugins").join(id).join("data").join(file)
    }

    #[test]
    fn a_member_gets_a_program_in_its_own_directory_with_nothing_of_the_helpers_environment() {
        let dir = tempfile::tempdir().unwrap();
        let member = plugin(
            dir.path(),
            "env-plugin",
            "#!/bin/sh\nenv > env.out\npwd > pwd.out\nexec sleep 30\n",
            my_uid(),
        );
        let mut runner = Runner::new(dir.path().join("plugins"), shared(dir.path()));
        runner.sync(&roster(vec![member]));
        assert_eq!(runner.running(), vec!["env-plugin".to_owned()]);
        assert!(wait(|| data(dir.path(), "env-plugin", "pwd.out").exists()));
        assert!(wait(|| !std::fs::read_to_string(data(
            dir.path(),
            "env-plugin",
            "env.out"
        ))
        .unwrap_or_default()
        .is_empty()));
        let env = std::fs::read_to_string(data(dir.path(), "env-plugin", "env.out")).unwrap();
        assert!(
            env.lines().any(|l| l == "LUMATV_PLUGIN=env-plugin"),
            "{env}"
        );
        assert!(env.contains("HOME="), "{env}");
        assert!(
            !env.lines()
                .any(|l| l.starts_with("CARGO") || l.starts_with("RUST")),
            "the helper's environment is not the plugin's: {env}"
        );
        let pwd = std::fs::read_to_string(data(dir.path(), "env-plugin", "pwd.out")).unwrap();
        assert!(pwd.trim().ends_with("plugins/env-plugin/data"), "{pwd}");
        assert_eq!(
            std::fs::metadata(dir.path().join("plugins/env-plugin/data"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        runner.stop_all(Duration::from_secs(2));
        assert!(runner.running().is_empty());
    }

    #[test]
    fn a_plugin_that_is_switched_off_is_told_to_finish_and_is_ended() {
        let dir = tempfile::tempdir().unwrap();
        let member = plugin(
            dir.path(),
            "polite",
            "#!/bin/sh\ntrap 'echo told > told.out; exit 0' TERM\necho up > up.out\nwhile :; do sleep 0.1; done\n",
            my_uid(),
        );
        // The helper blocks the termination signals in every thread it starts (`sys::block_termination_signals`), and the thread
        // that starts a program is one of them: the program must not inherit that.
        // SAFETY: a signal set made and used in this thread only.
        let blocked = unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGTERM);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
            set
        };
        let mut runner = Runner::new(dir.path().join("plugins"), shared(dir.path()));
        runner.sync(&roster(vec![member]));
        assert!(wait(|| data(dir.path(), "polite", "up.out").exists()));
        runner.sync(&roster(Vec::new()));
        // SAFETY: undoes the block above.
        unsafe { libc::pthread_sigmask(libc::SIG_UNBLOCK, &blocked, std::ptr::null_mut()) };
        assert!(runner.running().is_empty());
        assert!(
            data(dir.path(), "polite", "told.out").exists(),
            "SIGTERM reached it, which it asked to hear before it was ended"
        );
    }

    #[test]
    fn a_program_that_stops_is_started_again_after_pauses_that_grow() {
        let dir = tempfile::tempdir().unwrap();
        let member = plugin(
            dir.path(),
            "dying",
            "#!/bin/sh\necho x >> starts.out\nexit 3\n",
            my_uid(),
        );
        let backoff = Backoff {
            first: Duration::from_millis(60),
            longest: Duration::from_millis(500),
            stable: Duration::from_secs(30),
        };
        let mut runner =
            Runner::with_backoff(dir.path().join("plugins"), shared(dir.path()), backoff);
        runner.sync(&roster(vec![member]));
        let started = Instant::now();
        let count = || {
            std::fs::read_to_string(data(dir.path(), "dying", "starts.out"))
                .map_or(0, |t| t.lines().count())
        };
        assert!(wait(|| {
            runner.tick();
            count() >= 4
        }));
        // 60 + 120 + 240 ms of pause lie between the first and the fourth start: it did not spin.
        assert!(
            started.elapsed() >= Duration::from_millis(400),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(pause(backoff, 1), Duration::from_millis(60));
        assert_eq!(pause(backoff, 3), Duration::from_millis(240));
        assert_eq!(
            pause(backoff, 10),
            Duration::from_millis(500),
            "and not for ever"
        );
        runner.stop_all(Duration::from_secs(1));
    }

    #[test]
    fn a_program_that_is_not_the_one_the_owner_installed_is_not_started_and_the_plugin_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let mut member = plugin(
            dir.path(),
            "swapped",
            "#!/bin/sh\necho x > ran.out\n",
            my_uid(),
        );
        member.program = Some("0".repeat(64));
        let shared = shared(dir.path());
        let mut runner = Runner::new(dir.path().join("plugins"), shared.clone());
        shared.set_roster(&roster(vec![member.clone()]), None);
        runner.sync(&roster(vec![member.clone()]));
        assert!(runner.running().is_empty());
        let topic = shared.hub.get("plugins").unwrap();
        let reason = topic["plugins"][0]["reason"].as_str().unwrap();
        assert!(
            reason.contains("not the one that was installed"),
            "{reason}"
        );
        assert!(!data(dir.path(), "swapped", "ran.out").exists());

        // The same file with its true hash runs.
        let mut text = Vec::new();
        std::fs::File::open(dir.path().join("plugins/swapped/bin/run.sh"))
            .unwrap()
            .read_to_end(&mut text)
            .unwrap();
        member.program = Some(sha256_of(&text));
        let mut runner = Runner::new(dir.path().join("plugins"), shared);
        runner.sync(&roster(vec![member]));
        assert!(wait(|| data(dir.path(), "swapped", "ran.out").exists()));
        runner.stop_all(Duration::from_secs(1));
    }

    fn sha256_of(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn a_program_anyone_can_write_or_that_leaves_its_directory_is_not_started() {
        let dir = tempfile::tempdir().unwrap();
        let member = plugin(
            dir.path(),
            "loose",
            "#!/bin/sh\necho x > ran.out\n",
            my_uid(),
        );
        let program = dir.path().join("plugins/loose/bin/run.sh");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o777)).unwrap();
        let package = dir.path().join("plugins/loose");
        assert!(
            checked_program(&package, "bin/run.sh", None)
                .unwrap_err()
                .contains("can be changed")
        );
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(checked_program(&package, "bin/run.sh", None).is_ok());
        // A directory the program sits in that others can write is as bad.
        std::fs::set_permissions(package.join("bin"), std::fs::Permissions::from_mode(0o775))
            .unwrap();
        assert!(
            checked_program(&package, "bin/run.sh", None)
                .unwrap_err()
                .contains("can be changed")
        );
        std::fs::set_permissions(package.join("bin"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        // A link out of the directory is not the plugin's program.
        std::os::unix::fs::symlink("/bin/sh", package.join("bin/out")).unwrap();
        assert!(
            checked_program(&package, "bin/out", None)
                .unwrap_err()
                .contains("outside")
        );
        assert!(checked_program(&package, "bin/none", None).is_err());
        drop(member);
    }

    #[test]
    fn as_root_a_program_runs_as_the_plugins_user_and_group_and_cannot_gain_more() {
        if !crate::sys::is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let member = plugin(
            dir.path(),
            "boxed",
            "#!/bin/sh\n{ id -u; id -g; id -G; grep NoNewPrivs /proc/self/status; ulimit -n; } > who.out 2>&1\nexec sleep 30\n",
            20007,
        );
        let mut runner = Runner::new(dir.path().join("plugins"), shared(dir.path()));
        runner.sync(&roster(vec![member]));
        assert!(wait(|| std::fs::read_to_string(data(
            dir.path(),
            "boxed",
            "who.out"
        ))
        .is_ok_and(|t| t.lines().count() >= 5)));
        let who = std::fs::read_to_string(data(dir.path(), "boxed", "who.out")).unwrap();
        let lines: Vec<&str> = who.lines().collect();
        assert_eq!(lines[0], "20007", "{who}");
        assert_eq!(lines[1], "2000", "{who}");
        assert_eq!(lines[2], "2000", "no other group: {who}");
        assert!(lines[3].ends_with('1'), "no_new_privs: {who}");
        assert_eq!(lines[4], "64", "{who}");
        runner.stop_all(Duration::from_secs(1));
    }
}
