//! The kernel files the policy reads and writes: `/proc` and LG's cgroup v1 hierarchies, both under roots a test points at a
//! made-up tree.

use super::pressure::{self, Sample};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

/// A process as the policy names it: the number, and the tick it started at, so a reused number is never mistaken for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Pid {
    pub pid: i32,
    pub start: u64,
}

/// LumaTV's cpu cgroup for cached apps. Its existence at the helper's start is the trace of a run that did not restore.
pub const BACKGROUND_GROUP: &str = "lumatv-background";
/// Android's `SCHED_GROUP_BACKGROUND` weight against the 1024 of every task left in the root group.
pub const BACKGROUND_SHARES: u32 = 52;

pub trait Os: Send + Sync {
    /// The processes of an app not owned by root, from SAM's memory cgroup of the app; `None` when the app has no such group.
    fn app_procs(&self, app: &str) -> Option<Vec<Pid>>;
    fn alive(&self, p: Pid) -> bool;
    fn oom(&self, p: Pid) -> Option<i32>;
    fn set_oom(&self, p: Pid, value: i32) -> io::Result<()>;
    /// Every thread of the process is in the root cpu group: the only place it is moved from, so going back is exact.
    fn cpu_root(&self, p: Pid) -> bool;
    fn to_background(&self, p: Pid) -> io::Result<()>;
    fn to_root(&self, pid: i32) -> io::Result<()>;
    /// Moves whatever is left in the background group back and removes the group.
    fn drop_background(&self) -> io::Result<()>;
    fn freezer(&self, app: &str) -> Option<String>;
    fn set_freezer(&self, app: &str, state: &str) -> io::Result<()>;
    fn soft_limit(&self, app: &str) -> Option<u64>;
    fn set_soft_limit(&self, app: &str, bytes: u64) -> io::Result<()>;
    /// The app's charged memory, to choose among equally old apps.
    fn usage(&self, app: &str) -> u64;
    /// Reclaims what the app's memory cgroup holds (`memory.force_empty`); blocks until the kernel has done what it can.
    fn reclaim(&self, app: &str) -> io::Result<()>;
    fn sample(&self) -> Option<Sample>;
    /// `MemAvailable` of `/proc/meminfo`, in kB.
    fn available_kb(&self) -> Option<u64>;
    /// `Cached` + `Buffers` + `SReclaimable` of `/proc/meminfo`, in kB: what `drop_caches` can release.
    fn cache_kb(&self) -> Option<u64>;
    /// Free memory in contiguous blocks of at least `min_order` (4 KiB pages, 2^order each) from `/proc/buddyinfo`, in kB.
    fn contiguous_kb(&self, min_order: usize) -> Option<u64>;
    /// What the app's processes hold, as `/proc/<pid>/status` says; what closing the app gives back.
    fn footprint(&self, app: &str) -> Footprint;
    /// Writes one of the kernel's `/proc/sys/vm` knobs (`compact_memory`, `drop_caches`); blocks while the kernel does it.
    fn vm(&self, name: &str, value: &str) -> io::Result<()>;

    // What LG's memchute does and the policy takes over while LumaTV's memory manager serves (`manager.rs`).

    /// The kernel's `usable_pages` (LG's `low_mem_notify`), the number LG's levels are judged on.
    fn usable_pages(&self) -> Option<u64>;
    /// Every process in the app's memory group, root's too; `None` when it has no group.
    fn group_pids(&self, app: &str) -> Option<Vec<i32>>;
    /// Makes the app's memory and freezer groups where missing.
    fn make_groups(&self, app: &str) -> io::Result<()>;
    /// Moves a process into the app's memory and freezer groups; `Ok(false)` when it was in both.
    fn attach(&self, app: &str, pid: i32) -> io::Result<bool>;
    /// Removes the app's memory and freezer groups; a group with a process left in it stays.
    fn remove_groups(&self, app: &str);
    fn set_swappiness(&self, app: &str, value: u32) -> io::Result<()>;
    /// Asks the daemons LG's systrim configuration names to trim their heaps (`/tmp/systrim/<pid>/trim`); how many were asked.
    fn systrim(&self) -> usize;
}

/// The kernel and LG files outside `/proc` and the cgroup roots.
pub struct Extra {
    /// `usable_pages`.
    pub usable: PathBuf,
    /// The directories of the processes that take a trim request.
    pub systrim: PathBuf,
    /// LG's list of the daemons that take one.
    pub systrim_conf: PathBuf,
}

impl Extra {
    pub fn lg() -> Self {
        Self {
            usable: PathBuf::from("/sys/kernel/mm/low_mem_notify/usable_pages"),
            systrim: PathBuf::from("/tmp/systrim"),
            systrim_conf: PathBuf::from("/etc/palm/systrim-conf.json"),
        }
    }
}

/// The soft limit a v1 memory cgroup reads back when it has none; written as `-1`.
pub const UNLIMITED: u64 = 9_223_372_036_854_771_712;

pub struct Fs {
    proc: PathBuf,
    cgroup: PathBuf,
    extra: Extra,
}

impl Fs {
    pub fn new(proc: impl Into<PathBuf>, cgroup: impl Into<PathBuf>) -> Self {
        Self::with(proc, cgroup, Extra::lg())
    }

    pub fn with(proc: impl Into<PathBuf>, cgroup: impl Into<PathBuf>, extra: Extra) -> Self {
        Self {
            proc: proc.into(),
            cgroup: cgroup.into(),
            extra,
        }
    }

    pub fn lg() -> Self {
        Self::new("/proc", "/sys/fs/cgroup")
    }

    fn proc_file(&self, pid: i32, name: &str) -> PathBuf {
        self.proc.join(pid.to_string()).join(name)
    }

    fn memory(&self, app: &str, name: &str) -> PathBuf {
        self.cgroup.join("memory").join(app).join(name)
    }

    fn cpu(&self) -> PathBuf {
        self.cgroup.join("cpu,cpuacct")
    }

    fn started(&self, pid: i32) -> Option<u64> {
        let stat = fs::read_to_string(self.proc_file(pid, "stat")).ok()?;
        stat.get(stat.rfind(')')? + 1..)?
            .split_whitespace()
            .nth(19)?
            .parse()
            .ok()
    }

    fn uid(&self, pid: i32) -> Option<u32> {
        fs::read_to_string(self.proc_file(pid, "status"))
            .ok()?
            .lines()
            .find_map(|l| l.strip_prefix("Uid:"))?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    }

    fn same(&self, p: Pid) -> io::Result<()> {
        if self.started(p.pid) == Some(p.start) {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "the process is gone"))
        }
    }
}

fn read_trim(path: PathBuf) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

/// A cgroup v1 file takes one value per `write`.
fn put(path: PathBuf, value: &str) -> io::Result<()> {
    fs::write(path, value)
}

impl Os for Fs {
    fn app_procs(&self, app: &str) -> Option<Vec<Pid>> {
        let text = fs::read_to_string(self.memory(app, "cgroup.procs")).ok()?;
        Some(
            text.lines()
                .filter_map(|l| l.trim().parse::<i32>().ok())
                .filter(|&pid| pid > 1 && self.uid(pid).is_some_and(|u| u != 0))
                .filter_map(|pid| Some(Pid { pid, start: self.started(pid)? }))
                .collect(),
        )
    }

    fn alive(&self, p: Pid) -> bool {
        self.started(p.pid) == Some(p.start)
    }

    fn oom(&self, p: Pid) -> Option<i32> {
        self.same(p).ok()?;
        read_trim(self.proc_file(p.pid, "oom_score_adj"))?.parse().ok()
    }

    fn set_oom(&self, p: Pid, value: i32) -> io::Result<()> {
        self.same(p)?;
        put(self.proc_file(p.pid, "oom_score_adj"), &value.to_string())
    }

    fn cpu_root(&self, p: Pid) -> bool {
        let Ok(tasks) = fs::read_dir(self.proc_file(p.pid, "task")) else {
            return false;
        };
        let mut any = false;
        for task in tasks.flatten() {
            let Ok(text) = fs::read_to_string(task.path().join("cgroup")) else {
                return false;
            };
            let root = text.lines().any(|l| {
                let mut f = l.splitn(3, ':');
                f.nth(1).is_some_and(|c| c.split(',').any(|c| c == "cpu")) && f.next() == Some("/")
            });
            if !root {
                return false;
            }
            any = true;
        }
        any && self.alive(p)
    }

    fn to_background(&self, p: Pid) -> io::Result<()> {
        let group = self.cpu().join(BACKGROUND_GROUP);
        if !group.is_dir() {
            fs::create_dir(&group)?;
            put(group.join("cpu.shares"), &BACKGROUND_SHARES.to_string())?;
        }
        self.same(p)?;
        put(group.join("cgroup.procs"), &p.pid.to_string())
    }

    fn to_root(&self, pid: i32) -> io::Result<()> {
        put(self.cpu().join("cgroup.procs"), &pid.to_string())
    }

    fn drop_background(&self) -> io::Result<()> {
        let group = self.cpu().join(BACKGROUND_GROUP);
        let Ok(text) = fs::read_to_string(group.join("cgroup.procs")) else {
            return Ok(());
        };
        for pid in text.lines().filter_map(|l| l.trim().parse::<i32>().ok()) {
            let _ = self.to_root(pid);
        }
        fs::remove_dir(group)
    }

    fn freezer(&self, app: &str) -> Option<String> {
        read_trim(self.cgroup.join("freezer").join(app).join("freezer.state"))
    }

    fn set_freezer(&self, app: &str, state: &str) -> io::Result<()> {
        put(self.cgroup.join("freezer").join(app).join("freezer.state"), state)
    }

    fn soft_limit(&self, app: &str) -> Option<u64> {
        read_trim(self.memory(app, "memory.soft_limit_in_bytes"))?.parse().ok()
    }

    fn set_soft_limit(&self, app: &str, bytes: u64) -> io::Result<()> {
        let value = if bytes >= UNLIMITED { "-1".to_owned() } else { bytes.to_string() };
        put(self.memory(app, "memory.soft_limit_in_bytes"), &value)
    }

    fn usage(&self, app: &str) -> u64 {
        read_trim(self.memory(app, "memory.usage_in_bytes"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    fn reclaim(&self, app: &str) -> io::Result<()> {
        put(self.memory(app, "memory.force_empty"), "0")
    }

    fn sample(&self) -> Option<Sample> {
        let read = |name: &str| fs::read_to_string(self.proc.join(name)).ok();
        pressure::sample(
            &read("pressure/memory")?,
            &read("meminfo")?,
            &read("vmstat")?,
            &read("zoneinfo").unwrap_or_default(),
        )
    }

    fn available_kb(&self) -> Option<u64> {
        kb_of(&fs::read_to_string(self.proc.join("meminfo")).ok()?, "MemAvailable")
    }

    fn cache_kb(&self) -> Option<u64> {
        let text = fs::read_to_string(self.proc.join("meminfo")).ok()?;
        Some(kb_of(&text, "Cached")? + kb_of(&text, "Buffers")? + kb_of(&text, "SReclaimable").unwrap_or(0))
    }

    fn contiguous_kb(&self, min_order: usize) -> Option<u64> {
        let text = fs::read_to_string(self.proc.join("buddyinfo")).ok()?;
        let mut kb = 0;
        for line in text.lines() {
            // `Node 0, zone   Normal  <free blocks of order 0> <order 1> ...`
            let counts = line.split_once("zone")?.1.split_whitespace().skip(1);
            for (order, n) in counts.enumerate().skip(min_order) {
                kb += n.parse::<u64>().ok()? * (4 << order);
            }
        }
        Some(kb)
    }

    fn footprint(&self, app: &str) -> Footprint {
        let mut total = Footprint::default();
        for p in self.app_procs(app).unwrap_or_default() {
            let Some(status) = read_trim(self.proc_file(p.pid, "status")) else { continue };
            total.anon_kb += kb_of(&status, "RssAnon").or_else(|| kb_of(&status, "VmRSS")).unwrap_or(0);
            total.swap_kb += kb_of(&status, "VmSwap").unwrap_or(0);
        }
        total
    }

    fn vm(&self, name: &str, value: &str) -> io::Result<()> {
        put(self.proc.join("sys/vm").join(name), value)
    }

    fn usable_pages(&self) -> Option<u64> {
        read_trim(self.extra.usable.clone())?.parse().ok()
    }

    fn group_pids(&self, app: &str) -> Option<Vec<i32>> {
        let text = fs::read_to_string(self.memory(app, "cgroup.procs")).ok()?;
        Some(text.lines().filter_map(|l| l.trim().parse().ok()).collect())
    }

    fn make_groups(&self, app: &str) -> io::Result<()> {
        for root in ["memory", "freezer"] {
            fs::create_dir_all(self.cgroup.join(root).join(app))?;
        }
        Ok(())
    }

    fn attach(&self, app: &str, pid: i32) -> io::Result<bool> {
        let membership = fs::read_to_string(self.proc_file(pid, "cgroup"))?;
        let want = format!("/{app}");
        let mut moved = false;
        for root in ["memory", "freezer"] {
            // `<id>:<controllers>:<path>`; the memory and freezer hierarchies are each mounted alone on these sets.
            let inside = membership.lines().any(|l| {
                let mut f = l.splitn(3, ':');
                let (_, controllers, path) = (f.next(), f.next().unwrap_or(""), f.next().unwrap_or(""));
                controllers.split(',').any(|c| c == root) && path == want
            });
            if !inside {
                put(self.cgroup.join(root).join(app).join("cgroup.procs"), &pid.to_string())?;
                moved = true;
            }
        }
        Ok(moved)
    }

    fn remove_groups(&self, app: &str) {
        for root in ["memory", "freezer"] {
            let _ = fs::remove_dir(self.cgroup.join(root).join(app));
        }
    }

    fn set_swappiness(&self, app: &str, value: u32) -> io::Result<()> {
        put(self.memory(app, "memory.swappiness"), &value.to_string())
    }

    fn systrim(&self) -> usize {
        let Some(names) = fs::read_to_string(&self.extra.systrim_conf).ok().and_then(|t| serde_json::from_str(&t).ok()).map(|v| systrim_names(&v))
        else {
            return 0;
        };
        let Ok(dirs) = fs::read_dir(&self.extra.systrim) else { return 0 };
        let mut asked = 0;
        for dir in dirs.flatten() {
            let Some(pid) = dir.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
            let Some(comm) = read_trim(self.proc_file(pid, "comm")) else { continue };
            if names.contains(&comm) && fs::File::create(dir.path().join("trim")).is_ok() {
                asked += 1;
            }
        }
        asked
    }
}

/// Every name LG's systrim configuration holds, as a key or a string at any depth, cut to the 15 bytes a process's `comm` keeps
/// [inference: the file's layout was never read; this reads any].
fn systrim_names(v: &serde_json::Value) -> std::collections::BTreeSet<String> {
    fn add(s: &str, out: &mut std::collections::BTreeSet<String>) {
        let name = s.rsplit('/').next().unwrap_or(s);
        out.insert(name.chars().take(15).collect());
    }
    fn walk(v: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match v {
            serde_json::Value::String(s) => add(s, out),
            serde_json::Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
            serde_json::Value::Object(o) => {
                for (k, v) in o {
                    add(k, out);
                    walk(v, out);
                }
            }
            _ => {}
        }
    }
    let mut out = std::collections::BTreeSet::new();
    walk(v, &mut out);
    out
}

/// The memory of an app's processes in kB: `anon_kb` is what exiting returns to the system (`RssAnon`, or `VmRSS` on a kernel
/// without it), `swap_kb` what they hold in swap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Footprint {
    pub anon_kb: u64,
    pub swap_kb: u64,
}

/// One `Key:  <n> kB` line of a `/proc` text.
fn kb_of(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))?.split_whitespace().next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn process(proc: &Path, pid: i32, uid: u32, cpu: &str) {
        let dir = proc.join(pid.to_string());
        fs::create_dir_all(dir.join("task").join(pid.to_string())).unwrap();
        fs::write(dir.join("stat"), format!("{pid} (Plex Main (x)) S 1 {pid} {pid} 0 -1 4194560 1 0 0 0 0 0 0 0 20 0 44 0 777 0 0")).unwrap();
        fs::write(dir.join("status"), format!("Name:\tplex\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n")).unwrap();
        fs::write(dir.join("oom_score_adj"), "-1000\n").unwrap();
        fs::write(dir.join("task").join(pid.to_string()).join("cgroup"), format!("7:memory:/app\n3:cpu,cpuacct:{cpu}\n")).unwrap();
    }

    #[test]
    fn an_apps_processes_come_from_its_memory_group_without_roots() {
        let dir = tempfile::tempdir().unwrap();
        let (proc, cg) = (dir.path().join("proc"), dir.path().join("cg"));
        process(&proc, 300, 5365, "/");
        process(&proc, 301, 0, "/");
        fs::create_dir_all(cg.join("memory/app")).unwrap();
        fs::write(cg.join("memory/app/cgroup.procs"), "300\n301\n302\n").unwrap();
        let os = Fs::new(&proc, &cg);
        assert_eq!(os.app_procs("app"), Some(vec![Pid { pid: 300, start: 777 }]));
        assert_eq!(os.app_procs("other"), None);
        let p = Pid { pid: 300, start: 777 };
        assert_eq!(os.oom(p), Some(-1000));
        assert!(os.cpu_root(p));
        assert!(os.set_oom(Pid { pid: 300, start: 778 }, 900).is_err());
        os.set_oom(p, 900).unwrap();
        assert_eq!(os.oom(p), Some(900));
    }

    #[test]
    fn a_process_with_a_thread_outside_the_root_cpu_group_is_not_moved() {
        let dir = tempfile::tempdir().unwrap();
        let proc = dir.path().join("proc");
        process(&proc, 300, 5365, "/update-3152");
        assert!(!Fs::new(&proc, dir.path()).cpu_root(Pid { pid: 300, start: 777 }));
    }

    #[test]
    fn the_background_group_is_made_with_androids_weight_and_removed_with_its_members_sent_back() {
        let dir = tempfile::tempdir().unwrap();
        let (proc, cg) = (dir.path().join("proc"), dir.path().join("cg"));
        process(&proc, 300, 5365, "/");
        fs::create_dir_all(cg.join("cpu,cpuacct")).unwrap();
        let os = Fs::new(&proc, &cg);
        os.to_background(Pid { pid: 300, start: 777 }).unwrap();
        let group = cg.join("cpu,cpuacct").join(BACKGROUND_GROUP);
        assert_eq!(fs::read_to_string(group.join("cpu.shares")).unwrap(), "52");
        assert_eq!(fs::read_to_string(group.join("cgroup.procs")).unwrap(), "300");
        // A real cgroup directory removes its own files; the test's must be emptied first.
        let _ = os.drop_background();
        assert_eq!(fs::read_to_string(cg.join("cpu,cpuacct/cgroup.procs")).unwrap(), "300");
    }

    #[test]
    fn an_unlimited_soft_limit_is_written_back_as_minus_one() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("memory/app")).unwrap();
        let os = Fs::new(dir.path(), dir.path());
        os.set_soft_limit("app", UNLIMITED).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("memory/app/memory.soft_limit_in_bytes")).unwrap(), "-1");
        os.set_soft_limit("app", 0).unwrap();
        assert_eq!(os.soft_limit("app"), Some(0));
    }

    #[test]
    fn available_memory_is_read_and_vm_knobs_are_written_under_proc() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("sys/vm")).unwrap();
        fs::write(dir.path().join("meminfo"), "MemTotal:  1952000 kB\nMemFree:  100 kB\nMemAvailable:  612340 kB\n").unwrap();
        let os = Fs::new(dir.path(), dir.path());
        assert_eq!(os.available_kb(), Some(612_340));
        os.vm("drop_caches", "1").unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("sys/vm/drop_caches")).unwrap(), "1");
    }

    #[test]
    fn page_cache_free_blocks_and_an_apps_own_memory_are_read_from_proc() {
        let dir = tempfile::tempdir().unwrap();
        let (proc, cg) = (dir.path().join("proc"), dir.path().join("cg"));
        fs::create_dir_all(&proc).unwrap();
        fs::write(proc.join("meminfo"), "MemAvailable:  600 kB\nBuffers:  10 kB\nCached:  200 kB\nSwapCached:  999 kB\nSReclaimable:  30 kB\n").unwrap();
        // Orders 0..=5 in two zones: order 4 is 64 kB, order 5 is 128 kB.
        fs::write(proc.join("buddyinfo"), "Node 0, zone      DMA     9    9    9    9    1    0\nNode 0, zone   Normal    50   40   30   20    3    2\n").unwrap();
        process(&proc, 300, 5365, "/");
        fs::write(proc.join("300/status"), "Name:\tplex\nUid:\t5365\t5365\t5365\t5365\nVmRSS:\t 900 kB\nRssAnon:\t 400 kB\nVmSwap:\t 70 kB\n").unwrap();
        process(&proc, 301, 5365, "/");
        fs::write(proc.join("301/status"), "Name:\tplex\nUid:\t5365\t5365\t5365\t5365\nVmRSS:\t 50 kB\n").unwrap();
        fs::create_dir_all(cg.join("memory/app")).unwrap();
        fs::write(cg.join("memory/app/cgroup.procs"), "300\n301\n").unwrap();
        let os = Fs::new(&proc, &cg);
        assert_eq!(os.cache_kb(), Some(240));
        assert_eq!(os.contiguous_kb(4), Some(4 * 64 + 2 * 128));
        assert_eq!(os.contiguous_kb(0), Some(604 + 2088));
        // The second process has no `RssAnon` (an older kernel): its `VmRSS` stands in.
        assert_eq!(os.footprint("app"), Footprint { anon_kb: 450, swap_kb: 70 });
        assert_eq!(os.footprint("other"), Footprint::default());
        fs::remove_file(proc.join("buddyinfo")).unwrap();
        assert_eq!(os.contiguous_kb(4), None);
    }

    #[test]
    fn groups_are_made_a_process_is_attached_once_and_named_daemons_are_asked_to_trim() {
        let dir = tempfile::tempdir().unwrap();
        let (proc, cg) = (dir.path().join("proc"), dir.path().join("cg"));
        process(&proc, 300, 5365, "/");
        fs::write(proc.join("300/cgroup"), "9:freezer:/\n7:memory:/\n").unwrap();
        let extra = Extra { usable: dir.path().join("usable"), systrim: dir.path().join("systrim"), systrim_conf: dir.path().join("conf.json") };
        fs::write(&extra.usable, "218002\n").unwrap();
        let os = Fs::with(&proc, &cg, extra);
        assert_eq!(os.usable_pages(), Some(218_002));
        os.make_groups("app").unwrap();
        assert!(os.attach("app", 300).unwrap());
        assert_eq!(fs::read_to_string(cg.join("freezer/app/cgroup.procs")).unwrap(), "300");
        assert_eq!(os.group_pids("app"), Some(vec![300]));
        fs::write(proc.join("300/cgroup"), "9:freezer:/app\n7:memory:/app\n").unwrap();
        assert!(!os.attach("app", 300).unwrap());
        os.set_swappiness("app", 10).unwrap();
        assert_eq!(fs::read_to_string(cg.join("memory/app/memory.swappiness")).unwrap(), "10");

        fs::write(dir.path().join("conf.json"), r#"{"whitelist":["/usr/sbin/surface-manager","ls-hubd"]}"#).unwrap();
        for (pid, comm) in [(300, "surface-manager"), (301, "plex")] {
            fs::create_dir_all(dir.path().join(format!("systrim/{pid}"))).unwrap();
            fs::create_dir_all(proc.join(pid.to_string())).unwrap();
            fs::write(proc.join(format!("{pid}/comm")), format!("{comm}\n")).unwrap();
        }
        assert_eq!(os.systrim(), 1);
        assert!(dir.path().join("systrim/300/trim").exists() && !dir.path().join("systrim/301/trim").exists());
    }
}
